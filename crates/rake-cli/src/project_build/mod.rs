//! Native project builds, Web deployment and development supervision.
//! The native HTTP origin owns application assets, the broker and preview transports.

mod args;
mod blaze_install;
#[cfg(any(unix, windows))]
mod broker;
mod browser;
mod bundle;
#[cfg(any(unix, windows))]
mod dev;
mod favicon;
mod offline;
mod permissions;
#[cfg(any(unix, windows))]
mod preview;
mod processes;
#[cfg(any(unix, windows))]
mod resources;
#[cfg(any(unix, windows))]
mod serve;
mod web;
#[cfg(windows)]
mod windows;

use args::{Args, Compiler};
use processes::{Capture, Processes};
use serde_json::{Value, json};

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Keep an inherited-console client and its descendants owned on Windows.
#[cfg(windows)]
pub(crate) fn run_client(command: &mut Command) -> Result<i32> {
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancelled);

    ctrlc::set_handler(move || flag.store(true, Ordering::Release))?;
    let mut child = Processes::new(&cancelled).start(command)?;

    loop {
        if cancelled.load(Ordering::Acquire) {
            // Dropping the owned Job also stops a client which ignores Ctrl-C.
            return Ok(130);
        }

        if let Some(status) = child.0.try_wait()? {
            return Ok(status.code().unwrap_or(1));
        }

        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

pub(crate) fn run(command: &str, arguments: &[String]) -> i32 {
    let mut args = match args::parse(command, arguments) {
        Ok(Some(args)) => args,
        Ok(None) => return 0,
        Err(error) => {
            args::report_error(arguments, &error);

            return 2;
        }
    };

    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancelled);

    let result = ctrlc::set_handler(move || flag.store(true, Ordering::Release))
        .map_err(Into::into)
        .and_then(|()| {
            if args.live {
                args.blaze_binary = Some(blaze_install::ensure()?);
            }
            let processes = Processes::new(&cancelled);

            if command == "doctor" {
                doctor(&args, &processes)
            } else if command == "dev" {
                dev::run(&args, &cancelled).map(|()| 0)
            } else {
                build(&args, &processes).map(|()| 0)
            }
        });

    if cancelled.load(Ordering::Acquire) {
        return 0;
    }

    match result {
        Ok(code) => code,
        Err(error) => {
            if args.json {
                report_json_error(&error.to_string());
            } else {
                eprintln!("rake: {error}");
            }

            1
        }
    }
}

fn doctor(args: &Args, processes: &Processes<'_>) -> Result<i32> {
    #[derive(serde::Serialize)]
    struct Checks {
        rust: Value,
        cargo: Value,
        installed_targets: Value,
        wasm_bindgen: Value,
        node: Value,
        blaze: Value,
        live: Value,
    }
    let probe = |program: &str, arguments: &[&str]| {
        let mut command = Command::new(program);

        command.args(arguments);

        match processes.capture(&mut command, Capture::Diagnostic) {
            Ok(output) => json!(output.trim()),
            Err(error) => json!({"error":error.to_string()}),
        }
    };

    let checks = Checks {
        live: blaze_install::readiness(),
        blaze: match probe("blaze", &["--version"]) {
            Value::String(version) => json!({"note":version}),
            _ => json!({"note":"optional: Blaze not found; Cargo remains the default compiler"}),
        },
        rust: probe("rustc", &["--version"]),
        cargo: probe("cargo", &["--version"]),
        installed_targets: probe("rustup", &["target", "list", "--installed"]),
        wasm_bindgen: probe("wasm-bindgen", &["--version"]),
        node: probe("node", &["--version"]),
    };

    let ready = checks.rust.is_string()
        && checks.cargo.is_string()
        && (!args.web
            || (checks.wasm_bindgen.is_string()
                && checks
                    .installed_targets
                    .to_string()
                    .contains("wasm32-unknown-unknown")));

    println!(
        "{}",
        if args.json {
            json_line(&checks)?
        } else {
            ascii_json(&serde_json::to_string_pretty(&checks)?)
        }
    );

    Ok(if ready { 0 } else { 1 })
}

#[derive(Clone)]
struct Project {
    package: Value,
    target: String,
    root: PathBuf,
    web_dir: PathBuf,
    assets: Option<PathBuf>,
    bindgen: Option<PathBuf>,
    watch_roots: Vec<PathBuf>,
    target_directory: PathBuf,
}

impl Project {
    fn load(args: &Args, processes: &Processes<'_>) -> Result<Self> {
        let metadata_command = |no_deps, locked| {
            let mut command = Command::new("cargo");

            command.args(["metadata", "--format-version", "1"]);

            if no_deps {
                command.arg("--no-deps");
            }

            if locked {
                command.arg("--locked");
            }

            command.arg("--manifest-path").arg(&args.manifest);

            command
        };

        let workspace: Value = serde_json::from_str(
            &processes.capture(&mut metadata_command(true, false), Capture::Metadata)?,
        )?;

        let root = PathBuf::from(string(&workspace, "workspace_root")?);

        let metadata: Value = serde_json::from_str(&processes.capture(
            &mut metadata_command(false, root.join("Cargo.lock").is_file()),
            Capture::Metadata,
        )?)?;

        let root = PathBuf::from(string(&metadata, "workspace_root")?);

        let members = metadata["workspace_members"]
            .as_array()
            .ok_or("Cargo workspace members missing")?;

        let mut packages: Vec<_> = metadata["packages"]
            .as_array()
            .ok_or("Cargo packages missing")?
            .iter()
            .filter(|package| members.contains(&package["id"]))
            .collect();

        if let Some(package) = &args.package {
            packages.retain(|item| item["name"] == *package);
        } else {
            let manifest = args
                .manifest
                .canonicalize()
                .unwrap_or_else(|_| args.manifest.clone());

            let exact: Vec<_> = packages
                .iter()
                .copied()
                .filter(|item| {
                    item["manifest_path"].as_str().is_some_and(|path| {
                        let path = Path::new(path);

                        path.canonicalize().unwrap_or_else(|_| path.to_owned()) == manifest
                    })
                })
                .collect();

            if !exact.is_empty() {
                packages = exact;
            }
        }

        if packages.len() != 1 {
            return Err(format!(
                "select one application with --package: {}",
                packages
                    .iter()
                    .filter_map(|item| item["name"].as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .into());
        }

        let package = packages[0];

        let directory = Path::new(string(package, "manifest_path")?)
            .parent()
            .ok_or("package directory missing")?;

        let assets = web::bundle_assets(package, directory, !args.web)?;
        let available_targets = package["targets"]
            .as_array()
            .ok_or("Cargo targets missing")?;
        let targets: Vec<_> = available_targets
            .iter()
            .filter(|target| {
                let kinds = if args.web {
                    &target["crate_types"]
                } else {
                    &target["kind"]
                };

                kinds.as_array().is_some_and(|kinds| {
                    kinds
                        .iter()
                        .any(|kind| kind == if args.web { "cdylib" } else { "bin" })
                }) && args
                    .binary
                    .as_ref()
                    .is_none_or(|name| target["name"] == *name)
            })
            .collect();
        if targets.is_empty() && !args.web {
            let binaries: Vec<_> = available_targets
                .iter()
                .filter(|target| {
                    target["kind"]
                        .as_array()
                        .is_some_and(|kinds| kinds.iter().any(|kind| kind == "bin"))
                })
                .filter_map(|target| target["name"].as_str())
                .collect();
            let name = string(package, "name")?;

            if binaries.is_empty() {
                return Err(format!(
                    "package '{name}' has no native binary target; add src/main.rs or [[bin]] to use rake dev (including --live); a cdylib alone is not a native app"
                )
                .into());
            }

            return Err(format!(
                "package '{name}' has no binary named '{}'; available binaries: {}",
                args.binary.as_deref().unwrap_or_default(),
                binaries.join(", ")
            )
            .into());
        }
        if targets.len() != 1 {
            return Err("select one binary with --bin, or a single cdylib for Web".into());
        }

        let target = string(targets[0], "name")?.to_owned();

        let web_dir = resolve(
            &args
                .web_dir
                .clone()
                .unwrap_or_else(|| directory.join("web")),
        )?;

        if args.web_dir.is_some() && !web_dir.is_dir() {
            return Err("--web-dir must name an existing directory".into());
        }

        let bindgen = if args.web {
            let lock: toml::Value = toml::from_str(&fs::read_to_string(root.join("Cargo.lock"))?)?;

            let version = lock["package"]
                .as_array()
                .and_then(|packages| {
                    packages
                        .iter()
                        .find(|package| package["name"].as_str() == Some("wasm-bindgen"))
                })
                .and_then(|package| package["version"].as_str())
                .ok_or("wasm-bindgen missing from Cargo.lock")?;

            let bindgen = processes::which("wasm-bindgen");

            let matches = if let Some(path) = &bindgen {
                processes
                    .capture(Command::new(path).arg("--version"), Capture::Version)?
                    .trim()
                    == format!("wasm-bindgen {version}")
            } else {
                false
            };

            if !matches {
                return Err(format!("install matching loader tooling: cargo install wasm-bindgen-cli --version {version} --locked").into());
            }

            bindgen
        } else {
            None
        };

        let watch_roots = {
            let mut watch_roots = vec![root.clone()];

            for package in metadata["packages"]
                .as_array()
                .ok_or("Cargo packages missing")?
            {
                if package["source"].is_null() {
                    let path = Path::new(string(package, "manifest_path")?);

                    if let Some(directory) = path.parent() {
                        watch_roots.push(directory.to_owned());
                    }
                }
            }

            watch_roots.sort();
            watch_roots.dedup();

            watch_roots
        };

        Ok(Self {
            target_directory: metadata["target_directory"]
                .as_str()
                .map(PathBuf::from)
                .unwrap_or_else(|| root.join("target")),
            watch_roots,
            package: package.clone(),
            target,
            root,
            web_dir,
            assets,
            bindgen,
        })
    }

    fn build_features(&self, args: &Args) -> Option<String> {
        let mut features: Vec<String> = args.features.iter().cloned().collect();

        if args.dev {
            for feature in ["devtools"]
                .into_iter()
                .chain((args.compiler == Compiler::Blaze).then_some("live"))
            {
                if self.package["features"].get(feature).is_some() {
                    features.push(feature.into());
                } else if let Some(dependencies) = self.package["dependencies"].as_array() {
                    for dependency in dependencies {
                        if dependency["kind"].is_null()
                            && ["rake-app", "rak", "rake"]
                                .iter()
                                .any(|name| dependency["name"] == *name)
                            && let Some(name) = dependency["rename"]
                                .as_str()
                                .or_else(|| dependency["name"].as_str())
                        {
                            features.push(format!("{name}/{feature}"));
                        }
                    }
                }
            }
        }

        (!features.is_empty()).then(|| features.join(","))
    }

    fn build(&self, args: &Args, processes: &Processes<'_>, output: &Path) -> Result<PathBuf> {
        if args.web {
            return web::build(self, args, processes, output);
        }

        let executable = if args.dev {
            std::env::var_os("RAKE_CLI")
                .map(PathBuf::from)
                .unwrap_or(std::env::current_exe()?)
        } else {
            std::env::current_exe()?
        };

        let mut command = Command::new(executable);

        command
            .args([
                "bundle",
                "--native",
                "--compat",
                "--json",
                "--manifest-path",
            ])
            .arg(string(&self.package, "manifest_path")?)
            .arg("--package")
            .arg(string(&self.package, "name")?)
            .arg("--bin")
            .arg(&self.target)
            .arg("--output")
            .arg(output)
            .arg(if args.release { "--release" } else { "--debug" });

        if let Some(features) = self.build_features(args) {
            command.arg("--features").arg(features);
        }

        let result: Value =
            serde_json::from_str(&processes.capture(&mut command, Capture::Metadata)?)?;

        let artifact = PathBuf::from(string(&result, "artifact")?);

        Ok(if cfg!(target_os = "macos") {
            artifact.join("Contents/MacOS").join(&self.target)
        } else {
            artifact
        })
    }
}

fn build(args: &Args, processes: &Processes<'_>) -> Result<()> {
    if let Some(reloader) = &args.native_reloader {
        reloader.canonicalize()?;
    }

    let project = Project::load(args, processes)?;
    let target = if args.web { "web" } else { "desktop" };
    let name = string(&project.package, "name")?;

    let output = args.output.clone().unwrap_or_else(|| {
        project
            .root
            .join("dist")
            .join(name)
            .join(if args.bundle { "web-bundle" } else { target })
    });

    let mut marker = json!({"package":name,"target":target});

    if args.bundle {
        marker["command"] = json!("bundle");
    }

    let inputs = project
        .assets
        .iter()
        .map(PathBuf::as_path)
        .chain(args.web.then_some(project.web_dir.as_path()));

    for input in inputs {
        if output.starts_with(input) || input.starts_with(&output) {
            return Err("output must not contain or overlap an asset/Web input directory".into());
        }
    }

    let marker_path = output.join(".rake-build.json");

    if output.exists()
        && (!marker_path.is_file()
            || marker_path.is_symlink()
            || serde_json::from_slice::<Value>(&fs::read(&marker_path)?)? != marker)
    {
        return Err(format!(
            "output contains files not owned by this build: {}; choose --output",
            output.display()
        )
        .into());
    }

    let parent = output.parent().ok_or("output parent missing")?;

    fs::create_dir_all(parent)?;
    let staging = tempfile::Builder::new()
        .prefix(".rake-build-")
        .tempdir_in(parent)?;

    fs::remove_dir(staging.path())?;
    let staged_artifact = if args.bundle {
        bundle::distribution(&project, args, processes, staging.path())?
    } else {
        project.build(args, processes, staging.path())?
    };

    processes.check_cancelled()?;
    write_marker(&staging.path().join(".rake-build.json"), &marker)?;
    let relative = staged_artifact.strip_prefix(staging.path())?;

    let artifact = if relative.as_os_str().is_empty() {
        output.clone()
    } else {
        output.join(relative)
    };

    let backup = output.with_file_name(format!(
        ".{}-{}.old",
        output
            .file_name()
            .ok_or("output name missing")?
            .to_string_lossy(),
        uuid::Uuid::new_v4()
    ));

    if output.exists() {
        fs::rename(&output, &backup)?;
    }

    if let Err(error) = fs::rename(staging.path(), &output) {
        if backup.exists() {
            fs::rename(&backup, &output)?;
        }

        return Err(error.into());
    }

    let _ = fs::remove_dir_all(&backup);

    if args.bundle {
        return bundle::report(&output, &artifact, args.json);
    }

    if args.json {
        println!(
            "{{\"artifact\": {}}}",
            ascii_json(&json!(artifact).to_string())
        );
    } else {
        println!("Built {}", artifact.display());
    }

    Ok(())
}

fn write_marker(path: &Path, marker: &Value) -> Result<()> {
    use std::io::Write;
    #[cfg(unix)]
    let mut file = tempfile::NamedTempFile::new_in(path.parent().ok_or("marker parent missing")?)?;

    #[cfg(windows)]
    let mut file = tempfile::Builder::new().make_in(
        path.parent().ok_or("marker parent missing")?,
        rake_client::private::create,
    )?;

    file.write_all(&serde_json::to_vec(marker)?)?;
    #[cfg(unix)]
    file.persist(path)?;
    #[cfg(windows)]
    rake_client::private::replace(file.as_file(), path)?;

    Ok(())
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .ok_or_else(|| format!("missing {key} in tool metadata").into())
}

/// Resolve existing symlink ancestors as pathlib.resolve does, including new outputs.
fn resolve(path: &Path) -> std::io::Result<PathBuf> {
    use std::path::Component;
    let absolute = std::path::absolute(path)?;
    let mut resolved = PathBuf::new();

    for component in absolute.components() {
        match component {
            Component::ParentDir => {
                resolved.pop();
            }
            Component::CurDir => {}
            other => {
                resolved.push(other);

                match fs::canonicalize(&resolved) {
                    Ok(path) => resolved = path,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
        }
    }

    Ok(resolved)
}

fn report_json_error(message: &str) {
    eprintln!(
        "{{\"error\": {{\"message\": {}}}}}",
        ascii_json(&json!(message).to_string())
    );
}

fn json_line(value: &impl serde::Serialize) -> Result<String> {
    // Match json.dumps separators while keeping doctor fields in their original order.
    struct Formatter;

    impl serde_json::ser::Formatter for Formatter {
        fn begin_array_value<W: ?Sized + std::io::Write>(
            &mut self,
            writer: &mut W,
            first: bool,
        ) -> std::io::Result<()> {
            if !first {
                writer.write_all(b", ")?;
            }

            Ok(())
        }

        fn begin_object_key<W: ?Sized + std::io::Write>(
            &mut self,
            writer: &mut W,
            first: bool,
        ) -> std::io::Result<()> {
            if !first {
                writer.write_all(b", ")?;
            }

            Ok(())
        }

        fn begin_object_value<W: ?Sized + std::io::Write>(
            &mut self,
            writer: &mut W,
        ) -> std::io::Result<()> {
            writer.write_all(b": ")
        }
    }
    let mut output = Vec::new();

    value.serialize(&mut serde_json::Serializer::with_formatter(
        &mut output,
        Formatter,
    ))?;

    Ok(ascii_json(&String::from_utf8(output)?))
}

fn ascii_json(text: &str) -> String {
    use std::fmt::Write;
    let mut output = String::new();

    for character in text.chars() {
        if character.is_ascii() {
            output.push(character);
        } else {
            for code in character.encode_utf16(&mut [0; 2]) {
                // Writing to a String is infallible.
                let _ = write!(output, "\\u{code:04x}");
            }
        }
    }

    output
}

pub(crate) fn web_target(arguments: &[String]) -> bool {
    args::web_target(arguments)
}

fn relative_name(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(all(test, windows))]
mod windows_private_tests {
    use super::*;

    #[test]
    fn development_markers_remain_private_when_atomically_replaced() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("output.json");

        for value in [
            json!({"generation":1}),
            json!({"generation":2,"text":"é漢字"}),
        ] {
            let previous = rake_client::private::open(&path).ok();

            write_marker(&path, &value).unwrap();

            if let Some(previous) = previous {
                assert_eq!(
                    serde_json::from_reader::<_, Value>(previous).unwrap(),
                    json!({"generation":1})
                );
            }

            let file = rake_client::private::open(&path).unwrap();

            assert_eq!(serde_json::from_reader::<_, Value>(file).unwrap(), value);
        }
    }
}
