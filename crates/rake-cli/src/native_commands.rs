//! Portable native compilation and read-only distribution diagnostics.
use std::{
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].as_str())
}
pub(crate) fn handles(command: &str, args: &[String]) -> bool {
    if value(args, "--target") == Some("web") {
        return false;
    }
    (command == "doctor" && args.iter().any(|arg| arg == "--distribution"))
        || (command == "build" && (cfg!(windows) || crate::distribution::handles(args)))
}
pub(crate) fn run(command: &str, args: &[String]) -> Result<()> {
    if command == "doctor" {
        return doctor(args);
    }
    let work = tempfile::tempdir()?;
    let output = work.path().join("build.json");
    let mut process = Command::new(std::env::current_exe()?);
    process.arg("__native-build").arg(&output).args(args);
    let flag = Arc::new(AtomicBool::new(false));
    let cancelled = Arc::clone(&flag);
    ctrlc::set_handler(move || cancelled.store(true, std::sync::atomic::Ordering::Release))?;
    let mut worker = rake_dist::Worker::spawn(&mut process)?;
    if !worker.wait(&flag, Duration::from_secs(1800))?.success() {
        return Err("native Cargo build failed".into());
    }
    println!("{}", std::fs::read_to_string(output)?);
    Ok(())
}
pub(crate) fn build_worker(output: &Path, args: &[String]) -> Result<()> {
    let mut command = Command::new("cargo");
    command.args([
        "build",
        "--locked",
        "--message-format=json-render-diagnostics",
    ]);
    let mut arguments = args.iter();
    while let Some(arg) = arguments.next() {
        match arg.as_str() {
            "--json" | "--native" => {}
            "--target" => {
                if arguments.next().is_none_or(|value| value != "desktop") {
                    return Err("native build requires --target desktop".into());
                }
            }
            "--debug" => {}
            "--release" => {
                command.arg("--release");
            }
            "--manifest-path" | "--package" | "-p" | "--bin" | "--features" => {
                command
                    .arg(arg)
                    .arg(arguments.next().ok_or("Cargo option requires a value")?);
            }
            "--target-triple" => {
                command
                    .arg("--target")
                    .arg(arguments.next().ok_or("--target-triple needs a value")?);
            }
            "--platform" => {
                let platform = arguments.next().ok_or("--platform needs a value")?;
                if platform != std::env::consts::OS {
                    return Err("native compilation for another platform requires an explicit --target-triple and installed toolchain".into());
                }
            }
            _ => return Err(format!("unsupported native build argument: {arg}").into()),
        }
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let mut reader = BufReader::new(child.stdout.take().ok_or("Cargo output missing")?);
    let mut artifacts = Vec::new();
    loop {
        let mut line = String::new();
        let count = (&mut reader)
            .take(4 * 1024 * 1024 + 1)
            .read_line(&mut line)?;
        if count == 0 {
            break;
        }
        if count > 4 * 1024 * 1024 {
            child.kill()?;
            child.wait()?;
            return Err("Cargo JSON message exceeds bound".into());
        }
        let message: serde_json::Value = serde_json::from_str(&line)?;
        if message["reason"] == "compiler-message"
            && let Some(rendered) = message["message"]["rendered"].as_str()
        {
            eprint!("{rendered}");
        }
        if message["reason"] == "compiler-artifact"
            && let Some(path) = message["executable"].as_str()
        {
            artifacts.push(PathBuf::from(path));
        }
    }
    if !child.wait()?.success() {
        return Err("Cargo rejected native build".into());
    }
    let primary = if artifacts.len() == 1 {
        Some(artifacts[0].clone())
    } else {
        None
    };
    std::fs::write(
        output,
        serde_json::to_vec_pretty(
            &serde_json::json!({"artifact":primary,"executables":artifacts}),
        )?,
    )?;
    Ok(())
}
fn tool_available(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|path| {
            path.join(name).is_file()
                || (cfg!(windows) && path.join(format!("{name}.exe")).is_file())
        })
    })
}
fn doctor(args: &[String]) -> Result<()> {
    let mut manifest = PathBuf::from("Cargo.toml");
    let mut package = None;
    let mut target = None;
    let mut platform = std::env::consts::OS;
    let mut formats = Vec::new();
    let mut arguments = args.iter();
    while let Some(arg) = arguments.next() {
        match arg.as_str() {
            "--distribution" | "--json" => {}
            "--manifest-path" => {
                manifest = arguments
                    .next()
                    .ok_or("--manifest-path needs a value")?
                    .into()
            }
            "--package" | "-p" => {
                package = Some(arguments.next().ok_or("--package needs a value")?.as_str())
            }
            "--target-triple" => {
                target = Some(
                    arguments
                        .next()
                        .ok_or("--target-triple needs a value")?
                        .as_str(),
                )
            }
            "--platform" => platform = arguments.next().ok_or("--platform needs a value")?,
            "--format" => formats.push(serde_json::from_value(serde_json::Value::String(
                arguments.next().ok_or("--format needs a value")?.clone(),
            ))?),
            _ => return Err(format!("unsupported distribution doctor argument: {arg}").into()),
        }
    }
    let platform_value = match platform {
        "macos" => rake_dist::Platform::Macos,
        "windows" => rake_dist::Platform::Windows,
        "linux" => rake_dist::Platform::Linux,
        _ => return Err("unsupported desktop platform".into()),
    };
    let host = rake_dist::host_target()?;
    let plan = rake_dist::plan_with(
        &manifest,
        package,
        platform_value,
        target.or((platform == std::env::consts::OS).then_some(host.as_str())),
        (!formats.is_empty()).then_some(formats.as_slice()),
    )?;
    let capabilities=plan.packages().iter().map(|item|{
        use rake_dist::Format;
        let mut tools=vec!["cargo","rustc"];
        match item.format(){Format::App|Format::Zip if platform=="macos"=>tools.extend(["codesign","ditto"]),Format::Dmg=>tools.extend(["codesign","hdiutil"]),Format::Msix=>{tools.extend(["makeappx","signtool","mt"]);if !item.config().desktop.localizations.is_empty(){tools.push("makepri");}},Format::Velopack=>tools.extend(["dotnet","vpk","signtool"]),Format::Nsis=>tools.push("makensis"),Format::Msi=>tools.push("candle"),Format::Deb=>tools.extend(["dpkg-deb","readelf","patchelf"]),Format::Rpm=>tools.extend(["readelf","patchelf"]),Format::Arch=>tools.extend(["makepkg","readelf","patchelf"]),Format::Flatpak=>tools.extend(["flatpak","flatpak-builder"]),Format::Appimage=>tools.extend(["readelf","patchelf"]),_=>{}}
        if item.config().release.repositories.values().any(|formats| formats.contains(&item.format())) {
            tools.push("gpg");
            match item.format() {
                Format::Deb => tools.extend(["dpkg-scanpackages", "apt-ftparchive"]),
                Format::Rpm => tools.extend(["rpmsign", "createrepo_c"]),
                Format::Arch => tools.push("repo-add"),
                Format::Flatpak => tools.push("ostree"),
                _ => {}
            }
        }
        let tools=tools.into_iter().map(|name|serde_json::json!({"name":name,"on_path":tool_available(name)})).collect::<Vec<_>>();
        serde_json::json!({"format":item.format(),"update_owner":item.update_owner(),"native_host_required":platform,"host_matches":platform==std::env::consts::OS,"tools":tools,"native_validation":"see docs/distribution-native-tests.md; tool presence does not prove installation support"})
    }).collect::<Vec<_>>();
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"host_target":host,"plan":plan,"capabilities":capabilities,"tool_resolution":"PATH probes; packager-managed downloads and Windows SDK enrollment are checked during bundle","component_inputs":"resolved and verified during bundle; doctor performs no downloads"})
        )?
    );
    Ok(())
}
