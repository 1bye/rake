//! Staged native jobs. The CLI runs this coordinator inside an owned worker.
use crate::{Error, Format, Metadata, Platform, Worker, engine, files, plan_metadata_with};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::AtomicBool,
    time::Duration,
};

/// Serializable request passed to an isolated packaging worker; contains no secrets.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BundleRequest {
    /// Cargo manifest path.
    pub manifest: PathBuf,
    /// Explicit workspace package name.
    pub package: Option<String>,
    /// Native packaging platform.
    pub platform: Platform,
    /// Rust target; absent means the compiler host.
    pub target: Option<String>,
    /// Explicit output selection; empty uses configuration.
    pub formats: Vec<Format>,
    /// Output directory owned by this application.
    pub output: PathBuf,
    /// Cargo binary target, required if the package has several binaries.
    pub binary: Option<String>,
    /// Build the release Cargo profile.
    pub release: bool,
}

/// Final artifact with deterministic integrity information (not an update signature).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Artifact {
    /// Path relative to the distribution directory.
    pub path: PathBuf,
    /// Package format.
    pub format: Format,
    /// SHA-256 of the file or deterministic directory inventory.
    pub sha256: String,
    /// Native package version.
    pub package_version: String,
}

/// Versioned result for every requested native artifact.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Distribution {
    /// Distribution manifest revision.
    pub schema_version: u32,
    /// Application identity.
    pub identifier: String,
    /// Cargo version.
    pub version: String,
    /// Rust target triple.
    pub target: String,
    /// Engine name and pinned version.
    pub engine: String,
    /// Final artifact records.
    pub artifacts: Vec<Artifact>,
}

/// Reads the compiler's native target triple.
/// # Errors
/// Returns process, compiler output or missing host errors.
pub fn host_target() -> Result<String, Error> {
    let output = Command::new("rustc").arg("-vV").output()?;
    if !output.status.success() {
        return Err(Error::Execution("rustc -vV failed".into()));
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("host: ").map(str::to_owned))
        .ok_or_else(|| Error::Execution("rustc host missing".into()))
}

/// Runs one bounded worker and reads its committed result.
/// # Errors
/// Returns worker failure, cancellation, timeout or invalid output errors.
pub fn bundle(
    request: &BundleRequest,
    executable: &Path,
    cancelled: &AtomicBool,
) -> Result<Distribution, Error> {
    let output = std::path::absolute(&request.output)?;
    let parent = output
        .parent()
        .ok_or_else(|| Error::Execution("output requires parent".into()))?;
    fs::create_dir_all(parent)?;
    // Parent owns staging so a forcibly terminated worker cannot leak its job.
    let workspace = tempfile::Builder::new()
        .prefix(".rake-worker-")
        .tempdir_in(parent)?;
    let request_file = workspace.path().join("request.json");
    files::write_json(&request_file, request)?;
    let mut command = Command::new(executable);
    command
        .arg("__distribution-worker")
        .arg(&request_file)
        .arg(workspace.path());
    let mut worker = Worker::spawn(&mut command)?;
    let status = worker.wait(cancelled, Duration::from_secs(1800))?;
    if !status.success() {
        return Err(Error::Execution(format!("worker exited {status}")));
    }
    Ok(serde_json::from_slice(&fs::read(
        request.output.join("distribution.json"),
    )?)?)
}

/// Executes a request in the current process. Call only in a dedicated worker.
/// # Errors
/// Returns build, confinement, engine and transactional-output errors.
pub fn execute_worker(request: &BundleRequest, job_parent: &Path) -> Result<Distribution, Error> {
    if request.platform != native_platform()? {
        return Err(Error::Execution(format!(
            "{:?} packaging requires a native runner",
            request.platform
        )));
    }
    let target = request.target.clone().map_or_else(host_target, Ok)?;
    let metadata_output = Command::new("cargo")
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--locked",
            "--offline",
            "--manifest-path",
        ])
        .arg(&request.manifest)
        .output()?;
    if !metadata_output.status.success() {
        return Err(Error::Cargo(
            String::from_utf8_lossy(&metadata_output.stderr).into(),
        ));
    }
    let plan = plan_metadata_with(
        &metadata_output.stdout,
        request.package.as_deref(),
        request.platform,
        Some(&target),
        (!request.formats.is_empty()).then_some(&request.formats),
    )?;
    let metadata: Metadata = serde_json::from_slice(&metadata_output.stdout)?;
    let selected = metadata
        .packages
        .iter()
        .find(|p| {
            metadata.workspace_members.contains(&p.id)
                && request.package.as_deref().is_none_or(|name| name == p.name)
        })
        .ok_or(Error::Package)?;
    let root = selected.manifest_path.parent().ok_or(Error::Package)?;
    let binaries: Vec<_> = selected
        .targets
        .iter()
        .filter(|t| {
            t.kind.iter().any(|k| k == "bin")
                && request.binary.as_deref().is_none_or(|name| name == t.name)
        })
        .collect();
    if binaries.len() != 1 {
        return Err(Error::Execution("select one executable with --bin".into()));
    }
    let binary = &binaries[0].name;
    let output = std::path::absolute(&request.output)?;
    let output_parent = output
        .parent()
        .ok_or_else(|| Error::Execution("output requires parent".into()))?;
    fs::create_dir_all(output_parent)?;
    let output = fs::canonicalize(output_parent)?.join(
        output
            .file_name()
            .ok_or_else(|| Error::Execution("output name".into()))?,
    );
    if root.starts_with(&output) {
        return Err(Error::Execution(
            "output overlaps application source".into(),
        ));
    }
    let parent = output
        .parent()
        .ok_or_else(|| Error::Execution("output requires parent".into()))?;
    fs::create_dir_all(parent)?;
    let lock_path = parent.join(format!(
        ".{}.rake-lock",
        output
            .file_name()
            .ok_or_else(|| Error::Execution("output name".into()))?
            .to_string_lossy()
    ));
    if lock_path.is_symlink() {
        return Err(Error::Execution("output lock is a symlink".into()));
    }
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path)?;
    lock.try_lock_exclusive()
        .map_err(|_| Error::Execution("another distribution job owns this output".into()))?;
    if output.exists()
        && (output.is_symlink()
            || fs::read_to_string(output.join(".rake-distribution"))
                .ok()
                .as_deref()
                != Some(plan.identifier()))
    {
        return Err(Error::Execution(
            "output is not owned by this application".into(),
        ));
    }
    let job = tempfile::Builder::new()
        .prefix(".rake-job-")
        .tempdir_in(job_parent)?;
    let result_dir = job.path().join("result");
    fs::create_dir(&result_dir)?;
    let built = build(
        &selected.manifest_path,
        &selected.name,
        binary,
        &target,
        request.release,
    )?;
    let mut artifacts = Vec::new();
    for (index, item) in plan.packages().iter().enumerate() {
        let stage = job.path().join(format!("package-{index}"));
        let bin = stage.join("bin");
        fs::create_dir_all(&bin)?;
        let suffix = if request.platform == Platform::Windows {
            ".exe"
        } else {
            ""
        };
        files::copy(&built, &bin.join(format!("{binary}{suffix}")))?;
        let config = item.config();
        let mut resources = HashMap::new();
        let resource_stage = stage.join("resources");
        fs::create_dir_all(&resource_stage)?;
        if let Some(assets) = &config.assets {
            let path = resource_stage.join("assets");
            files::copy(&files::confined(root, assets)?, &path)?;
            resources.insert(path.to_string_lossy().into_owned(), "assets/".into());
        }
        for resource in &config.resources {
            let path = resource_stage.join(&resource.to);
            files::copy(&files::confined(root, &resource.from)?, &path)?;
            resources.insert(
                path.to_string_lossy().into_owned(),
                resource.to.to_string_lossy().into_owned(),
            );
        }
        let mut helpers = Vec::new();
        for helper in &config.sidecars {
            let path = PathBuf::from(helper.path.to_string_lossy().replace("{target}", &target));
            files::copy(
                &files::confined(root, &path)?,
                &bin.join(format!("{}{suffix}", helper.name)),
            )?;
            helpers.push(helper.name.clone());
        }
        if !config.runtimes.is_empty() {
            return Err(Error::Execution(
                "runtime requests require a component recipe".into(),
            ));
        }
        let icons = config
            .icon
            .as_ref()
            .map(|path| files::confined(root, path).map(|p| vec![p.to_string_lossy().into_owned()]))
            .transpose()?
            .unwrap_or_default();
        let paths = engine::package(engine::Input {
            plan: &plan,
            index,
            bin: &bin,
            binary,
            target: &target,
            root,
            resources,
            icons,
            helpers,
        })?;
        for path in paths {
            if !fs::canonicalize(&path)?.starts_with(fs::canonicalize(&stage)?) {
                return Err(Error::Execution("engine output escaped job".into()));
            }
            let name = path
                .file_name()
                .ok_or_else(|| Error::Execution("artifact filename missing".into()))?;
            let relative = PathBuf::from(format!("{index}")).join(name);
            let destination = result_dir.join(&relative);
            fs::create_dir_all(
                destination
                    .parent()
                    .ok_or_else(|| Error::Execution("artifact parent".into()))?,
            )?;
            files::copy(&path, &destination)?;
            artifacts.push(Artifact {
                path: relative,
                format: item.format(),
                sha256: files::digest(&destination)?,
                package_version: item.version().version.clone(),
            });
        }
    }
    let result = Distribution {
        schema_version: 1,
        identifier: plan.identifier().into(),
        version: plan.version().into(),
        target,
        engine: "tauri-bundler 2.10.4".into(),
        artifacts,
    };
    files::write_json(&result_dir.join("distribution.json"), &result)?;
    files::commit(&result_dir, &output, plan.identifier())?;
    Ok(result)
}

pub(crate) fn native_platform() -> Result<Platform, Error> {
    match std::env::consts::OS {
        "macos" => Ok(Platform::Macos),
        "windows" => Ok(Platform::Windows),
        "linux" => Ok(Platform::Linux),
        _ => Err(Error::Execution("unsupported native host".into())),
    }
}

fn build(
    manifest: &Path,
    package: &str,
    binary: &str,
    target: &str,
    release: bool,
) -> Result<PathBuf, Error> {
    let mut command = Command::new("cargo");
    command
        .args([
            "build",
            "--locked",
            "--offline",
            "--message-format=json-render-diagnostics",
            "--manifest-path",
        ])
        .arg(manifest)
        .args(["--package", package, "--bin", binary, "--target", target]);
    if release {
        command.arg("--release");
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Execution("cargo output unavailable".into()))?;
    let mut executable = None;
    let mut reader = BufReader::new(stdout);
    loop {
        let mut line = String::new();
        let count = (&mut reader)
            .take(4 * 1024 * 1024 + 1)
            .read_line(&mut line)?;
        if count == 0 {
            break;
        }
        if count > 4 * 1024 * 1024 {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Execution("Cargo message exceeds bound".into()));
        }
        let message: serde_json::Value = serde_json::from_str(&line)?;
        if message["reason"] == "compiler-artifact"
            && message["target"]["name"] == binary
            && let Some(path) = message["executable"].as_str()
        {
            executable = Some(PathBuf::from(path));
        }
    }
    if !child.wait()?.success() {
        return Err(Error::Execution("Cargo build failed".into()));
    }
    executable.ok_or_else(|| Error::Execution("Cargo did not report an executable".into()))
}
