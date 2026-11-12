//! Native project builds, Web deployment and development supervision.
//! Broker and preview requests retain an owned Python compatibility backend during the port.

mod args;
mod bundle;
mod dev;
mod favicon;
mod offline;
mod processes;
mod serve;
mod web;

use args::Args;
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

pub(crate) fn run(command: &str, arguments: &[String]) -> i32 {
    let args = match args::parse(command, arguments) {
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
        python: Value,
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
        rust: probe("rustc", &["--version"]),
        cargo: probe("cargo", &["--version"]),
        installed_targets: probe("rustup", &["target", "list", "--installed"]),
        wasm_bindgen: probe("wasm-bindgen", &["--version"]),
        node: probe("node", &["--version"]),
        // Python remains an optional diagnostic while later phases still use it.
        python: match probe("python3", &["--version"]) {
            Value::String(version) => json!(version.split_whitespace().nth(1).unwrap_or(&version)),
            error => error,
        },
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

struct Project {
    package: Value,
    target: String,
    root: PathBuf,
    web_dir: PathBuf,
    assets: Option<PathBuf>,
    bindgen: Option<PathBuf>,
    watch_roots: Vec<PathBuf>,
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
        let targets: Vec<_> = package["targets"]
            .as_array()
            .ok_or("Cargo targets missing")?
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

        Ok(Self {
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
            if self.package["features"].get("devtools").is_some() {
                features.push("devtools".into());
            } else if let Some(dependencies) = self.package["dependencies"].as_array() {
                for dependency in dependencies {
                    if dependency["kind"].is_null()
                        && ["rake-app", "rak", "rake"]
                            .iter()
                            .any(|name| dependency["name"] == *name)
                    {
                        let name = dependency["rename"]
                            .as_str()
                            .or_else(|| dependency["name"].as_str());
                        if let Some(name) = name {
                            features.push(format!("{name}/devtools"));
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
    let mut file = tempfile::NamedTempFile::new_in(path.parent().ok_or("marker parent missing")?)?;
    file.write_all(&serde_json::to_vec(marker)?)?;
    file.persist(path)?;

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
