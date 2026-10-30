//! Portable read-only distribution planning, independent of Unix runtime IPC.

use rake_dist::Platform;
use std::path::PathBuf;

pub(crate) fn run(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
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
        return Ok(());
    }
    let mut manifest = PathBuf::from("Cargo.toml");
    let mut package = None;
    let mut platform = std::env::consts::OS;
    let mut target = None;
    let mut formats = Vec::new();
    let mut args = arguments.iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--plan" | "--json" => {}
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
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"plan": plan, "native_validation": "pending"})
        )?
    );
    Ok(())
}
