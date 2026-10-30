//! Portable read-only distribution planning, independent of Unix runtime IPC.

use rake_dist::Platform;
use std::path::PathBuf;

pub(crate) fn handles(arguments: &[String]) -> bool {
    if arguments.iter().any(|arg| {
        matches!(
            arg.as_str(),
            "--native"
                | "--plan"
                | "--schema"
                | "--platform"
                | "--target-triple"
                | "--format"
                | "--trusted"
                | "--notary-profile"
                | "--certificate-thumbprint"
        )
    }) {
        return true;
    }
    let value = |flag: &str| {
        arguments
            .windows(2)
            .find(|pair| pair[0] == flag)
            .map(|pair| pair[1].as_str())
    };
    let platform = match std::env::consts::OS {
        "macos" => Platform::Macos,
        "windows" => Platform::Windows,
        _ => Platform::Linux,
    };
    let target = rake_dist::host_target().ok();
    rake_dist::plan_with(
        std::path::Path::new(value("--manifest-path").unwrap_or("Cargo.toml")),
        value("--package").or_else(|| value("-p")),
        platform,
        target.as_deref(),
        None,
    )
    .is_ok_and(|plan| !plan.is_legacy())
}

pub(crate) fn run(arguments: &[String]) -> Result<bool, Box<dyn std::error::Error>> {
    if arguments.iter().any(|arg| arg == "--schema") {
        if arguments
            .iter()
            .any(|arg| !matches!(arg.as_str(), "--schema" | "--plan" | "--json"))
        {
            return Err("schema output accepts only --plan, --schema and --json".into());
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&rake_dist::configuration_schema())?
        );
        return Ok(true);
    }
    let planning = arguments.iter().any(|arg| arg == "--plan");
    let explicit = arguments.iter().any(|arg| {
        matches!(
            arg.as_str(),
            "--native" | "--format" | "--platform" | "--target-triple"
        )
    });
    let mut output = PathBuf::from("dist");
    let mut binary = None;
    let mut release = true;
    let mut offline_components = false;
    let mut locked_components = false;
    let mut trust = rake_dist::TrustOptions::default();
    let mut manifest = PathBuf::from("Cargo.toml");
    let mut package = None;
    let mut platform = std::env::consts::OS;
    let mut target = None;
    let mut formats = Vec::new();
    let mut features = Vec::new();
    let mut args = arguments.iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--plan" | "--json" | "--native" => {}
            "--release" => release = true,
            "--features" => features.extend(
                args.next()
                    .ok_or("--features requires a value")?
                    .split(',')
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned),
            ),
            "--trusted" => trust.required = true,
            "--sign-identity" => {
                trust.macos_identity = Some(
                    args.next()
                        .ok_or("--sign-identity requires a value")?
                        .clone(),
                )
            }
            "--notary-profile" => {
                trust.notary_profile = Some(
                    args.next()
                        .ok_or("--notary-profile requires a value")?
                        .clone(),
                )
            }
            "--certificate-thumbprint" => {
                trust.windows_thumbprint = Some(
                    args.next()
                        .ok_or("--certificate-thumbprint requires a value")?
                        .clone(),
                )
            }
            "--offline-components" => offline_components = true,
            "--locked-components" => locked_components = true,
            "--debug" => release = false,
            "--output" => output = args.next().ok_or("--output requires a value")?.into(),
            "--bin" => binary = Some(args.next().ok_or("--bin requires a value")?.clone()),
            "--manifest-path" => {
                manifest = args
                    .next()
                    .ok_or("--manifest-path requires a value")?
                    .into()
            }
            "--package" | "-p" => {
                package = Some(args.next().ok_or("--package requires a value")?.as_str())
            }
            "--platform" => platform = args.next().ok_or("--platform requires a value")?,
            "--target-triple" => {
                target = Some(
                    args.next()
                        .ok_or("--target-triple requires a value")?
                        .as_str(),
                )
            }
            "--format" => formats.push(serde_json::from_value::<rake_dist::Format>(
                serde_json::Value::String(args.next().ok_or("--format requires a value")?.clone()),
            )?),
            "--target" => {
                if args.next().is_none_or(|target| target != "desktop") {
                    return Err("native planning requires --target desktop".into());
                }
            }
            _ => {
                return Err(
                    format!("unsupported distribution planning argument: {argument}").into(),
                );
            }
        }
    }
    let platform = match platform {
        "macos" => Platform::Macos,
        "windows" => Platform::Windows,
        "linux" => Platform::Linux,
        _ => return Err(format!("unsupported platform: {platform}").into()),
    };
    let native_platform = match std::env::consts::OS {
        "macos" => Platform::Macos,
        "windows" => Platform::Windows,
        _ => Platform::Linux,
    };
    let host = if platform == native_platform && target.is_none() {
        Some(rake_dist::host_target()?)
    } else {
        None
    };
    let target = target.or(host.as_deref());
    let plan = rake_dist::plan_with(
        &manifest,
        package,
        platform,
        target,
        if formats.is_empty() {
            None
        } else {
            Some(&formats)
        },
    )?;
    if planning {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"plan": plan, "native_validation": "pending"})
            )?
        );
    } else if plan.is_legacy() && !explicit {
        return Ok(false);
    } else {
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = cancelled.clone();
        ctrlc::set_handler(move || flag.store(true, std::sync::atomic::Ordering::Release))?;
        let request = rake_dist::BundleRequest {
            manifest,
            package: package.map(str::to_owned),
            platform,
            target: target.map(str::to_owned),
            formats,
            output,
            binary,
            release,
            features,
            offline_components,
            locked_components,
            trust,
        };
        let result = rake_dist::bundle(&request, &std::env::current_exe()?, &cancelled)?;
        println!("{}", serde_json::to_string_pretty(&result)?);
    }
    Ok(true)
}

pub(crate) fn worker(
    path: &std::path::Path,
    workspace: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let request: rake_dist::BundleRequest = serde_json::from_slice(&std::fs::read(path)?)?;
    rake_dist::execute_worker(&request, workspace)?;
    Ok(())
}
