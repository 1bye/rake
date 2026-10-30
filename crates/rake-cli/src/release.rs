//! Explicit publication command, isolated from development IPC.
use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
#[derive(serde::Serialize, serde::Deserialize)]
struct Job {
    request: rake_dist::ReleaseRequest,
    repository_output: Option<PathBuf>,
    repositories: Vec<PathBuf>,
}
pub(crate) fn run(arguments: &[String]) -> Result<()> {
    let mut manifest = PathBuf::from("Cargo.toml");
    let mut package = None;
    let mut channel = None;
    let mut inputs = Vec::new();
    let mut planning = false;
    let mut repository_output = None;
    let mut repositories = Vec::new();
    let mut args = arguments.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--manifest-path" => {
                manifest = args.next().ok_or("--manifest-path needs a value")?.into()
            }
            "--package" | "-p" => {
                package = Some(args.next().ok_or("--package needs a value")?.clone())
            }
            "--channel" => channel = Some(args.next().ok_or("--channel needs a value")?.clone()),
            "--from" => inputs.push(PathBuf::from(
                args.next()
                    .ok_or("--from needs a bundle output directory")?,
            )),
            "--plan" => planning = true,
            "--prepare-repositories" => {
                repository_output = Some(std::path::absolute(
                    args.next()
                        .ok_or("--prepare-repositories needs a new output directory")?,
                )?)
            }
            "--repositories" => repositories
                .push(std::path::absolute(args.next().ok_or(
                    "--repositories needs a prepared snapshot directory",
                )?)?),
            "--json" => {}
            _ => return Err(format!("unsupported release argument: {arg}").into()),
        }
    }
    if repository_output.is_some() && !repositories.is_empty() {
        return Err("repository preparation cannot also publish prepared repositories".into());
    }
    let manifest = std::path::absolute(manifest)?;
    let metadata = std::process::Command::new("cargo")
        .args([
            "metadata",
            "--locked",
            "--offline",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(&manifest)
        .output()?;
    if !metadata.status.success() {
        return Err(String::from_utf8_lossy(&metadata.stderr)
            .into_owned()
            .into());
    }
    let value: serde_json::Value = serde_json::from_slice(&metadata.stdout)?;
    let members = value["workspace_members"]
        .as_array()
        .ok_or("Cargo members missing")?;
    let packages = value["packages"]
        .as_array()
        .ok_or("Cargo packages missing")?;
    let mut selected = packages.iter().filter(|item| {
        members.contains(&item["id"]) && package.as_ref().is_none_or(|name| item["name"] == *name)
    });
    let selected_package = selected.next().ok_or("select a workspace --package")?;
    if selected.next().is_some() {
        return Err("select one workspace --package".into());
    }
    let name = selected_package["name"]
        .as_str()
        .ok_or("Cargo package name missing")?;
    let package_root = Path::new(
        selected_package["manifest_path"]
            .as_str()
            .ok_or("Cargo manifest missing")?,
    )
    .parent()
    .ok_or("Cargo package root missing")?
    .to_owned();
    let target = rake_dist::host_target()?;
    let platform = match std::env::consts::OS {
        "macos" => rake_dist::Platform::Macos,
        "windows" => rake_dist::Platform::Windows,
        _ => rake_dist::Platform::Linux,
    };
    let plan =
        rake_dist::plan_metadata_with(&metadata.stdout, Some(name), platform, Some(&target), None)?;
    let config = plan.config();
    let channel = channel.unwrap_or_else(|| {
        config
            .updates
            .channel
            .clone()
            .unwrap_or_else(|| "stable".into())
    });
    if config.updates.enabled && config.updates.channel.as_deref().unwrap_or("stable") != channel {
        return Err("release channel conflicts with the compiled update channel; use a channel override and rebuild".into());
    }
    config.release.validate()?;
    if planning {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"plan":plan,"channel":channel,"inputs":inputs,"repositories":repositories,"repository_output":repository_output,"publication":"explicit release only","required":config.release.required})
            )?
        );
        return Ok(());
    }
    let flag = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&flag);
    ctrlc::set_handler(move || stop.store(true, std::sync::atomic::Ordering::Release))?;
    if inputs.is_empty() {
        let output = package_root.join("dist");
        rake_dist::bundle(
            &rake_dist::BundleRequest {
                manifest,
                package: Some(name.into()),
                platform,
                target: Some(target),
                formats: vec![],
                output: output.clone(),
                binary: None,
                release: true,
                compatibility: false,
                entitlements: None,
                features: vec![],
                offline_components: false,
                locked_components: true,
                trust: rake_dist::TrustOptions {
                    required: true,
                    ..Default::default()
                },
            },
            &std::env::current_exe()?,
            &flag,
        )?;
        inputs.push(output);
    }
    let mut keys = std::collections::BTreeMap::new();
    for item in plan.packages() {
        for (provider, path) in &item.config().updates.public_keys {
            let path = std::fs::canonicalize(package_root.join(path))?;
            if !path.starts_with(std::fs::canonicalize(&package_root)?) {
                return Err("release public key escapes package root".into());
            }
            let bytes = std::fs::read(path)?;
            if let Some(previous) = keys.insert(provider.clone(), bytes.clone())
                && previous != bytes
            {
                return Err("conflicting release public keys between formats".into());
            }
        }
    }
    let request = rake_dist::ReleaseRequest {
        identifier: plan.identifier().into(),
        version: plan.version().into(),
        channel,
        distributions: inputs
            .into_iter()
            .map(std::path::absolute)
            .collect::<std::io::Result<_>>()?,
        config: config.release.clone(),
        package_root,
        public_keys: keys,
        published_at: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        updates_enabled: config.updates.enabled,
        custom_verifiers: config
            .custom_formats
            .iter()
            .filter_map(|format| {
                format
                    .verify_command
                    .clone()
                    .map(|command| (format.name.clone(), command))
            })
            .collect(),
    };
    let work = tempfile::tempdir()?;
    let request_file = work.path().join("request.json");
    let result_file = work.path().join("result.json");
    std::fs::write(
        &request_file,
        serde_json::to_vec(&Job {
            request,
            repository_output,
            repositories,
        })?,
    )?;
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command
        .arg("__release-worker")
        .arg(request_file)
        .arg(&result_file);
    let mut worker = rake_dist::Worker::spawn(&mut command)?;
    if !worker.wait(&flag, Duration::from_secs(1800))?.success() {
        return Err(
            "release worker failed; previous channel retained unless promotion already completed"
                .into(),
        );
    }
    println!("{}", std::fs::read_to_string(result_file)?);
    Ok(())
}
pub(crate) fn worker(arguments: &[String]) -> Result<()> {
    if arguments.len() != 2 {
        return Err("release worker requires request/result paths".into());
    }
    let job: Job = serde_json::from_slice(&std::fs::read(&arguments[0])?)?;
    let flag = AtomicBool::new(false);
    let result = if let Some(output) = job.repository_output {
        rake_dist::prepare_repositories(&job.request, &output, &flag)?;
        serde_json::json!({"repositories":output,"publication":"not published"})
    } else {
        serde_json::to_value(rake_dist::publish_with_repositories(
            &job.request,
            &job.repositories,
            &flag,
        )?)?
    };
    std::fs::write(&arguments[1], serde_json::to_vec_pretty(&result)?)?;
    Ok(())
}
