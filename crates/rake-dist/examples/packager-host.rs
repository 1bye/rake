//! Minimal packager host: no rake-cli, application runtime, UI or build hook.
use std::{path::Path, sync::atomic::AtomicBool};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();

    if let Some(result) = rake_dist::dispatch_worker(&arguments) {
        result?;

        return Ok(());
    }

    let path = arguments
        .first()
        .ok_or("usage: packager-host Bundle.toml [target-triple] [--plan]")?;

    let source = rake_dist::load_bundle_file(Path::new(path))?;

    let target = arguments
        .get(1)
        .filter(|s| *s != "--plan")
        .map(|s| s.to_str().ok_or("target must be UTF-8"))
        .transpose()?
        .map(str::to_owned);

    let platform = target
        .as_deref()
        .map(rake_dist::Platform::from_target)
        .transpose()?
        .unwrap_or({
            if cfg!(target_os = "macos") {
                rake_dist::Platform::Macos
            } else if cfg!(windows) {
                rake_dist::Platform::Windows
            } else {
                rake_dist::Platform::Linux
            }
        });

    let mut request = rake_dist::SourceBundleRequest::new(source, platform);

    request.target = target;

    if arguments.iter().any(|s| s == "--plan") {
        println!(
            "{}",
            serde_json::to_string_pretty(&rake_dist::plan_source(&request)?)?
        );
    } else {
        let result =
            rake_dist::bundle_source(&request, &std::env::current_exe()?, &AtomicBool::new(false))?;

        println!("{}", serde_json::to_string_pretty(&result)?);
    }

    Ok(())
}
