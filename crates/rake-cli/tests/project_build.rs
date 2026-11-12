//! Covers: native doctor/build CLI contracts, Web artifacts, input safety and rollback.
//! Real: CLI dispatch, metadata parsing, filesystem publication and child processes.
//! Doubles: shell tools supply Cargo metadata/diagnostics and wasm-bindgen output.
//! Requires: Unix; no Python, Node, GPU, wasm target or network.

#![cfg(unix)]

use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
    process::{Command, Output},
};

struct Fixture {
    _directory: tempfile::TempDir,
    root: PathBuf,
    tools: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("app");
        let tools = directory.path().join("tools");
        fs::create_dir_all(root.join("web")).unwrap();
        fs::create_dir_all(root.join("assets/nested")).unwrap();
        fs::create_dir(&tools).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname='fixture'\nversion='1.0.0'\n",
        )
        .unwrap();
        fs::write(
            root.join("Cargo.lock"),
            "[[package]]\nname='wasm-bindgen'\nversion='0.2.100'\n",
        )
        .unwrap();
        fs::write(root.join("input.wasm"), b"\0asmfixture").unwrap();
        fs::write(root.join("assets/nested/message"), "asset").unwrap();
        let fixture = Self {
            _directory: directory,
            root,
            tools,
        };
        fixture.metadata(json!({"assets":"assets"}), 1);
        fixture.tool(
            "cargo",
            r#"case "$1" in
metadata) /bin/cat "$FIXTURE/metadata.json";;
build)
    printf '%s\n' "$@" > "$FIXTURE/build-args"
    if [ -f "$FIXTURE/fail" ]; then printf 'compiler failed\n' >&2; exit 7; fi
    printf 'cargo progress\n' >&2
    /bin/cat "$FIXTURE/artifacts.json"
    ;;
--version) printf 'cargo fixture\n';;
esac"#,
        );
        fixture.tool(
            "wasm-bindgen",
            r#"if [ "$1" = "--version" ]; then printf 'wasm-bindgen 0.2.100\n'; exit; fi
printf '%s\n' "$@" > "$FIXTURE/bindgen-args"
/bin/mkdir -p "$4"
/bin/cp "$5" "$4/fixture_bg.wasm"
printf 'export default async function init() {}' > "$4/fixture.js""#,
        );
        fixture.tool("rustc", "printf 'rustc fixture\n'");
        fixture.tool("rustup", "printf 'host-target\nwasm32-unknown-unknown\n'");
        fixture
    }

    fn metadata(&self, bundle: Value, count: usize) {
        let package = json!({
            "id":"fixture-id","name":"fixture","version":"1.0.0",
            "manifest_path":self.root.join("Cargo.toml"),
            "metadata":{"rake":{"bundle":bundle}},
            "targets":[{"name":"fixture","kind":["cdylib"],"crate_types":["cdylib"]}],
            "features":{},"dependencies":[],"source":null
        });
        let mut packages = vec![package];
        let mut members = vec!["fixture-id"];
        if count == 2 {
            let mut other = packages[0].clone();
            other["id"] = json!("other-id");
            other["name"] = json!("other");
            other["manifest_path"] = json!(self.root.join("other/Cargo.toml"));
            packages.push(other);
            members.push("other-id");
        }
        fs::write(
            self.root.join("metadata.json"),
            json!({
                "workspace_root":self.root,"workspace_members":members,"packages":packages
            })
            .to_string(),
        )
        .unwrap();
        fs::write(self.root.join("artifacts.json"), format!("{}\n{}\n",
            json!({"reason":"compiler-artifact","package_id":"unrelated","target":{"name":"fixture"},"filenames":["wrong.wasm"]}),
            json!({"reason":"compiler-artifact","package_id":"fixture-id","target":{"name":"fixture"},"filenames":[self.root.join("input.wasm")]}),
        )).unwrap();
    }

    fn tool(&self, name: &str, source: &str) {
        let path = self.tools.join(name);
        fs::write(&path, format!("#!/bin/sh\n{source}\n")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_rake"))
            .current_dir(&self.root)
            .env("PATH", &self.tools)
            .env("FIXTURE", &self.root)
            .args(args)
            .output()
            .unwrap()
    }

    fn build(&self, args: &[&str]) -> Output {
        let mut command = vec!["build", "--target", "web", "--json"];
        command.extend_from_slice(args);
        self.run(&command)
    }

    fn output(&self) -> PathBuf {
        self.root.join("dist/fixture/web")
    }
}

fn success(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn error(output: &Output, code: i32, expected: &str) {
    assert_eq!(output.status.code(), Some(code), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let value: Value =
        serde_json::from_str(stderr.lines().last().unwrap_or_default()).expect(&stderr);
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains(expected),
        "{value}"
    );
}

#[test]
fn doctor_matches_required_targets_and_optional_tools_without_python() {
    let fixture = Fixture::new();
    for args in [
        vec!["doctor", "--json"],
        vec!["doctor", "--target", "web", "--json"],
    ] {
        let result = success(&fixture.run(&args));
        assert_eq!(result["rust"], "rustc fixture");
        assert_eq!(
            result["installed_targets"],
            "host-target\nwasm32-unknown-unknown"
        );
        assert!(result["node"]["error"].is_string());
        assert!(result["python"]["error"].is_string());
    }
    fs::remove_file(fixture.tools.join("wasm-bindgen")).unwrap();
    assert!(fixture.run(&["doctor", "--json"]).status.success());
    let web = fixture.run(&["doctor", "--target", "web", "--json"]);
    assert_eq!(web.status.code(), Some(1));
    assert!(
        serde_json::from_slice::<Value>(&web.stdout).unwrap()["wasm_bindgen"]["error"].is_string()
    );
    fixture.tool("wasm-bindgen", "printf 'wasm-bindgen 0.2.100\n'");
    fixture.tool("rustup", "printf 'host-only\n'");
    assert_eq!(
        fixture.run(&["doctor", "--target", "web"]).status.code(),
        Some(1)
    );
    fixture.tool("rustc", "printf 'failure' >&2; exit 3");
    let result = fixture.run(&["doctor", "--json"]);
    assert_eq!(result.status.code(), Some(1));
    assert!(
        serde_json::from_slice::<Value>(&result.stdout).unwrap()["rust"]["error"]
            .as_str()
            .unwrap()
            .contains("exit status 3")
    );
}

#[test]
fn web_build_selects_artifact_copies_assets_and_preserves_explicit_features() {
    let fixture = Fixture::new();
    let result = fixture.build(&["--release", "--features", "theme,extra"]);
    let artifact = success(&result);
    assert_eq!(artifact["artifact"], json!(fixture.output()));
    assert_eq!(
        fs::read(fixture.output().join("pkg/fixture_bg.wasm")).unwrap(),
        b"\0asmfixture"
    );
    assert_eq!(
        fs::read_to_string(fixture.output().join("assets/nested/message")).unwrap(),
        "asset"
    );
    assert!(
        fs::read_to_string(fixture.output().join("index.html"))
            .unwrap()
            .contains("import init from '/pkg/fixture.js';await init();")
    );
    assert!(
        fixture
            .output()
            .join("rake-web-font-licenses.txt")
            .is_file()
    );
    assert_eq!(
        serde_json::from_slice::<Value>(
            &fs::read(fixture.output().join(".rake-build.json")).unwrap()
        )
        .unwrap(),
        json!({"package":"fixture","target":"web"})
    );
    assert_eq!(
        fs::metadata(fixture.output().join(".rake-build.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let args = fs::read_to_string(fixture.root.join("build-args")).unwrap();
    assert!(args.contains("--locked\n--message-format=json-render-diagnostics\n"));
    assert!(
        args.contains(
            "--features\ntheme,extra\n--release\n--lib\n--target\nwasm32-unknown-unknown\n"
        )
    );
    assert!(!args.contains("devtools"));
    assert!(String::from_utf8_lossy(&result.stderr).contains("cargo progress"));
}

#[test]
fn web_build_keeps_authored_index_and_rolls_back_failed_replacements() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join("web/index.html"), "<html>authored</html>").unwrap();
    success(&fixture.build(&[]));
    assert_eq!(
        fs::read_to_string(fixture.output().join("index.html")).unwrap(),
        "<html><link rel=\"icon\" type=\"image/svg+xml\" href=\"/assets/rake-icon.svg\">authored</html>"
    );
    fs::write(fixture.root.join("input.wasm"), "replacement").unwrap();
    fs::write(fixture.root.join("fail"), "").unwrap();
    let failed = fixture.build(&[]);
    assert_eq!(failed.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&failed.stderr).contains("cargo exited 7"));
    assert_eq!(
        fs::read(fixture.output().join("pkg/fixture_bg.wasm")).unwrap(),
        b"\0asmfixture"
    );
    fs::remove_file(fixture.root.join("fail")).unwrap();
    success(&fixture.build(&[]));
    assert_eq!(
        fs::read(fixture.output().join("pkg/fixture_bg.wasm")).unwrap(),
        b"replacement"
    );
    assert!(
        fs::read_dir(fixture.output().parent().unwrap())
            .unwrap()
            .all(|item| !item
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".rake-build-"))
    );
}

#[test]
fn build_rejects_unowned_output_overlap_and_symlink_markers() {
    let fixture = Fixture::new();
    let output = fixture.root.join("unrelated");
    fs::create_dir(&output).unwrap();
    fs::write(output.join("keep"), "user data").unwrap();
    error(
        &fixture.build(&["--output", output.to_str().unwrap()]),
        1,
        "not owned",
    );
    assert_eq!(
        fs::read_to_string(output.join("keep")).unwrap(),
        "user data"
    );
    for path in [
        fixture.root.join("assets/child"),
        fixture.root.join("assets"),
        fixture.root.clone(),
        fixture.root.join("web/child"),
    ] {
        error(
            &fixture.build(&["--output", path.to_str().unwrap()]),
            1,
            "overlap",
        );
    }
    let link = fixture.root.join("output-link");
    symlink(&output, &link).unwrap();
    error(
        &fixture.build(&["--output", link.to_str().unwrap()]),
        2,
        "output cannot be a symlink",
    );
    success(&fixture.build(&[]));
    fs::rename(
        fixture.output().join(".rake-build.json"),
        fixture.root.join("marker"),
    )
    .unwrap();
    symlink(
        fixture.root.join("marker"),
        fixture.output().join(".rake-build.json"),
    )
    .unwrap();
    error(&fixture.build(&[]), 1, "not owned");
}

#[test]
fn web_build_rejects_links_collisions_bad_metadata_and_loader_mismatches() {
    let fixture = Fixture::new();
    symlink(
        fixture.root.join("input.wasm"),
        fixture.root.join("web/link"),
    )
    .unwrap();
    error(&fixture.build(&[]), 1, "not a link or special file");
    fs::remove_file(fixture.root.join("web/link")).unwrap();
    let socket_path = fixture.root.join("web/socket");
    let socket = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
    error(&fixture.build(&[]), 1, "not a link or special file");
    drop(socket);
    fs::remove_file(socket_path).unwrap();
    fs::create_dir_all(fixture.root.join("web/assets/nested")).unwrap();
    error(&fixture.build(&[]), 1, "asset/output collision: nested");
    fs::remove_dir_all(fixture.root.join("web/assets")).unwrap();
    error(
        &fixture.build(&["--web-dir", "/nonexistent-rake-web-input"]),
        1,
        "--web-dir must name an existing directory",
    );
    fs::create_dir(fixture.root.join("web/pkg")).unwrap();
    error(&fixture.build(&[]), 1, "web/pkg is reserved");
    fs::remove_dir(fixture.root.join("web/pkg")).unwrap();
    fixture.metadata(json!({"identifier":"bad"}), 1);
    error(&fixture.build(&[]), 1, "bundle identifier");
    fixture.metadata(json!({"assets":"../"}), 1);
    error(&fixture.build(&[]), 1, "invalid bundle input");
    fixture.metadata(json!({"web":{"offline":"yes"}}), 1);
    error(&fixture.build(&[]), 1, "must be a boolean");
    fixture.metadata(json!({"typo":true}), 1);
    error(&fixture.build(&[]), 1, "unknown or invalid");
    fixture.metadata(json!({}), 1);
    fixture.tool("wasm-bindgen", "printf 'wasm-bindgen 0.2.99\n'");
    error(
        &fixture.build(&[]),
        1,
        "cargo install wasm-bindgen-cli --version 0.2.100 --locked",
    );
}

#[test]
fn web_build_reports_missing_artifacts_and_ambiguous_packages() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join("artifacts.json"), "").unwrap();
    error(
        &fixture.build(&[]),
        1,
        "cargo did not report an application artifact",
    );
    fs::write(fixture.root.join("artifacts.json"), format!("{}\n", json!({"reason":"compiler-artifact","package_id":"fixture-id","target":{"name":"fixture"},"filenames":["app.rlib"]}))).unwrap();
    error(
        &fixture.build(&[]),
        1,
        "cargo did not report a Wasm artifact",
    );
    fixture.metadata(json!({}), 2);
    error(
        &fixture.build(&["--manifest-path", "virtual.toml"]),
        1,
        "select one application with --package: fixture, other",
    );
    success(&fixture.build(&["--manifest-path", "virtual.toml", "-p", "fixture"]));
    error(
        &fixture.build(&["--bin", "absent"]),
        1,
        "select one binary with --bin",
    );
}

#[test]
fn parser_preserves_errors_flags_and_doctor_validation_order() {
    let fixture = Fixture::new();
    for (args, expected) in [
        (vec!["build", "--preview", "--json"], "--preview requires"),
        (vec!["build", "--cdp", "--json"], "--cdp applies"),
        (vec!["build", "--elements", "--json"], "--elements applies"),
        (vec!["build", "--port", "65536", "--json"], "port must be"),
        (
            vec!["build", "--unknown", "--json"],
            "unrecognized arguments",
        ),
        (
            vec!["build", "--release", "--debug", "--json"],
            "not allowed with",
        ),
        (
            vec!["build", "--target", "invalid", "--json"],
            "invalid choice",
        ),
        (
            vec!["doctor", "status", "--session", "/absent", "--json"],
            "use rake dev status",
        ),
    ] {
        error(&fixture.run(&args), 2, expected);
    }
    success(&fixture.run(&[
        "doctor",
        "--port",
        "65536",
        "--native-reloader",
        "/absent",
        "--json",
    ]));
    success(&fixture.run(&[
        "build",
        "--target=web",
        "--manifest-path=Cargo.toml",
        "--no-open",
        "--restart-without-state",
        "--instance=x",
        "--session=x",
        "--debug",
        "--json",
    ]));
    assert!(fixture.run(&["build", "--help"]).status.success());
}

#[test]
fn first_lock_resolution_and_subsequent_metadata_stay_locked() {
    let fixture = Fixture::new();
    fs::copy(
        fixture.root.join("Cargo.lock"),
        fixture.root.join("seed.lock"),
    )
    .unwrap();
    fs::remove_file(fixture.root.join("Cargo.lock")).unwrap();
    fixture.tool(
        "cargo",
        r#"if [ "$1" = metadata ]; then
printf '%s ' "$@" >> "$FIXTURE/metadata-args"
printf '\n' >> "$FIXTURE/metadata-args"
case " $* " in *" --no-deps "*) ;; *) /bin/cp "$FIXTURE/seed.lock" "$FIXTURE/Cargo.lock";; esac
/bin/cat "$FIXTURE/metadata.json"
else
/bin/cat "$FIXTURE/artifacts.json"
fi"#,
    );

    success(&fixture.build(&[]));
    let commands = fs::read_to_string(fixture.root.join("metadata-args")).unwrap();
    let commands: Vec<_> = commands.lines().collect();
    assert_eq!(commands.len(), 2);
    assert!(commands[0].contains("--no-deps"));
    assert!(!commands[1].contains("--locked"));

    success(&fixture.build(&[]));
    let commands = fs::read_to_string(fixture.root.join("metadata-args")).unwrap();
    assert!(commands.lines().last().unwrap().contains("--locked"));
}

#[test]
fn cancellation_returns_zero_cleans_descendants_and_keeps_published_output() {
    use std::time::{Duration, Instant};
    let fixture = Fixture::new();
    success(&fixture.build(&[]));
    fixture.tool(
        "cargo",
        r#"if [ "$1" = metadata ]; then /bin/cat "$FIXTURE/metadata.json"; exit; fi
(trap '' TERM; exec /bin/sleep 300) &
printf '%s' "$!" > "$FIXTURE/descendant"
wait"#,
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_rake"))
        .current_dir(&fixture.root)
        .env("PATH", &fixture.tools)
        .env("FIXTURE", &fixture.root)
        .args(["build", "--target", "web", "--json"])
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let pid: i32 = loop {
        if let Some(pid) = fs::read_to_string(fixture.root.join("descendant"))
            .ok()
            .and_then(|text| text.parse().ok())
        {
            break pid;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("build never started Cargo");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    // SAFETY: signal only the CLI child owned by this test.
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGTERM) }, 0);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            // SAFETY: clean up the fixture descendant on a failed cancellation.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
            panic!("build failed to stop after SIGTERM");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(status.success());
    // A killed orphan can remain a zombie until init reaps it.
    let state = Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let state = String::from_utf8_lossy(&state.stdout);
    assert!(
        state.trim().is_empty() || state.trim().starts_with('Z'),
        "descendant still running: {state}"
    );
    assert_eq!(
        fs::read(fixture.output().join("pkg/fixture_bg.wasm")).unwrap(),
        b"\0asmfixture"
    );
}

#[test]
fn real_desktop_build_does_not_execute_python() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::create_dir(root.join("tools")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='native-fixture'\nversion='0.1.0'\nedition='2024'\n[workspace]\n",
    )
    .unwrap();
    fs::write(
        root.join("src/main.rs"),
        "fn main(){println!(\"native works\");}",
    )
    .unwrap();
    let python = root.join("tools/python3");
    fs::write(
        &python,
        "#!/bin/sh\nprintf called > \"$FIXTURE/python-called\"\nexit 99\n",
    )
    .unwrap();
    fs::set_permissions(python, fs::Permissions::from_mode(0o755)).unwrap();
    let mut paths = vec![root.join("tools")];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let path = std::env::join_paths(paths).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rake"))
        .current_dir(&root)
        .env("PATH", path)
        .env("FIXTURE", &root)
        .env("CARGO_PROFILE_DEV_DEBUG", "0")
        .env_remove("CARGO_TARGET_DIR")
        .args(["build", "--debug", "--json"])
        .output()
        .unwrap();
    let result = success(&output);
    let executable = result["artifact"].as_str().unwrap();
    let run = Command::new(executable).output().unwrap();
    assert!(run.status.success());
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "native works\n");
    assert!(!root.join("python-called").exists());
    assert!(root.join("Cargo.lock").is_file());
    assert!(
        root.join("dist/native-fixture/desktop/.rake-build.json")
            .is_file()
    );
}

#[test]
fn symlinked_manifest_selects_exact_workspace_package() {
    let fixture = Fixture::new();
    fixture.metadata(json!({}), 2);
    let alias = fixture._directory.path().join("linked-app");
    symlink(fixture.root.canonicalize().unwrap(), &alias).unwrap();
    let manifest = alias.join("Cargo.toml");
    let mut metadata: Value =
        serde_json::from_slice(&fs::read(fixture.root.join("metadata.json")).unwrap()).unwrap();

    // Cargo can report the symlink spelling or its resolved path.
    for reported in [
        manifest.clone(),
        fixture.root.join("Cargo.toml").canonicalize().unwrap(),
    ] {
        metadata["packages"][0]["manifest_path"] = json!(reported);
        fs::write(fixture.root.join("metadata.json"), metadata.to_string()).unwrap();

        let result = success(&fixture.build(&["--manifest-path", manifest.to_str().unwrap()]));
        assert_eq!(result["artifact"], json!(fixture.output()));
    }

    // Synthetic/unavailable paths retain the raw comparison as a fallback.
    let missing = fixture.root.join("unavailable/Cargo.toml");
    metadata["packages"][0]["manifest_path"] = json!(missing);
    fs::write(fixture.root.join("metadata.json"), metadata.to_string()).unwrap();
    error(
        &fixture.build(&[
            "--manifest-path",
            missing.to_str().unwrap(),
            "--bin",
            "absent",
        ]),
        1,
        "select one binary with --bin",
    );
}

#[test]
fn help_and_argument_errors_use_rake_program_name() {
    let fixture = Fixture::new();
    for command in ["doctor", "build"] {
        let help = fixture.run(&[command, "--target", "web", "--help"]);
        assert!(help.status.success());
        assert!(help.stderr.is_empty());
        let text = String::from_utf8(help.stdout).unwrap();
        assert!(text.starts_with("usage: rake ["), "{text}");
        assert!(!text.contains("dev.py"), "{text}");

        let output = fixture.run(&[command, "--target", "web", "--unknown"]);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        let usage = text.split("\n\n").next().unwrap();
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            format!("{usage}\nrake: error: unrecognized arguments: --unknown\n"),
        );
    }
}

#[test]
fn web_default_favicon_preserves_authored_links_and_rejects_asset_collisions() {
    let fixture = Fixture::new();
    let index = fixture.root.join("web/index.html");
    let document = "<!-- <head><link rel='icon'> --><html><head><script>const x = '<link rel=icon>';</script></head></html>";
    fs::write(&index, document).unwrap();
    success(&fixture.build(&[]));
    let link = "<link rel=\"icon\" type=\"image/svg+xml\" href=\"/assets/rake-icon.svg\">";
    assert_eq!(
        fs::read_to_string(fixture.output().join("index.html")).unwrap(),
        document.replacen("<head><script>", &format!("<head>{link}<script>"), 1)
    );
    assert_eq!(
        fs::read(fixture.output().join("assets/rake-icon.svg")).unwrap(),
        include_bytes!("../scripts/default-icon.svg")
    );
    for authored in [
        "<link rel='icon' href='custom.svg'>",
        "<LINK REL='shortcut ICON' HREF='custom.ico'>",
        "<link rel=icon href='data:,'>",
        "<link href='custom.png' rel='alternate icon'/>",
        "<link rel='&#105;con' href='custom.svg'>",
    ] {
        let document = format!("<html><head>{authored}</head></html>");
        fs::write(&index, &document).unwrap();
        success(&fixture.build(&[]));
        assert_eq!(
            fs::read_to_string(fixture.output().join("index.html")).unwrap(),
            document
        );
        assert!(!fixture.output().join("assets/rake-icon.svg").exists());
    }
    fs::write(&index, "<html lang='en'><body>implicit head</body></html>").unwrap();
    success(&fixture.build(&[]));
    assert!(
        fs::read_to_string(fixture.output().join("index.html"))
            .unwrap()
            .contains(&format!("<html lang='en'>{link}"))
    );
    fs::write(
        fixture.root.join("assets/rake-icon.svg"),
        include_bytes!("../scripts/default-icon.svg"),
    )
    .unwrap();
    success(&fixture.build(&[]));
    fs::write(fixture.root.join("assets/rake-icon.svg"), "app owned").unwrap();
    let before = fs::read(fixture.output().join("index.html")).unwrap();
    error(&fixture.build(&[]), 1, "exists");
    assert_eq!(
        fs::read(fixture.output().join("index.html")).unwrap(),
        before
    );
    fs::remove_file(fixture.root.join("assets/rake-icon.svg")).unwrap();
    fs::write(&index, "fragment only").unwrap();
    error(
        &fixture.build(&[]),
        1,
        "Web document requires an html or head element",
    );
}

#[test]
fn web_bundle_preserves_manifest_archive_offline_payload_and_rollback() {
    use std::io::Read;
    let fixture = Fixture::new();
    fixture.metadata(json!({"assets":"assets","name":"Fixture","identifier":"dev.fixture.web","web":{"base_path":"/tools/","offline":true}}),1);
    let output = fixture.root.join("dist/fixture/web-bundle");
    let result = success(&fixture.run(&["bundle", "--target=web", "--json"]));
    assert_eq!(result["profile"], "release");
    assert_eq!(result["signing"], "not_applicable");
    assert_eq!(result["artifact"], json!(output.join("site")));
    let manifest: Value =
        serde_json::from_slice(&fs::read(output.join("bundle.json")).unwrap()).unwrap();
    let build = manifest["deployment"]["build"].as_str().unwrap();
    assert!(
        output
            .join(format!("site/rake-builds/{build}/pkg/fixture.js"))
            .is_file()
    );
    assert_eq!(manifest["deployment"]["base_path"], "/tools/");
    assert!(
        fs::read_to_string(fixture.root.join("build-args"))
            .unwrap()
            .contains("--release")
    );
    let mut archive =
        zip::ZipArchive::new(fs::File::open(output.join("fixture-1.0.0-web.zip")).unwrap())
            .unwrap();
    for entry in manifest["files"].as_array().unwrap() {
        let relative = entry["path"].as_str().unwrap();
        let source = fs::read(output.join(relative)).unwrap();
        assert_eq!(entry["bytes"], source.len());
        let mut content = Vec::new();
        let mut entry = archive
            .by_name(relative.strip_prefix("site/").unwrap())
            .unwrap();
        entry.read_to_end(&mut content).unwrap();
        assert_eq!(content, source);
        assert_eq!(entry.unix_mode().unwrap() & 0o777, 0o644);
        assert!(entry.last_modified().unwrap().year() >= 2026);
    }
    let kept = fs::read(output.join("bundle.json")).unwrap();
    fs::write(fixture.root.join("fail"), "fail").unwrap();
    error(
        &fixture.run(&["bundle", "--target", "web", "--json"]),
        1,
        "cargo exited 7",
    );
    assert_eq!(fs::read(output.join("bundle.json")).unwrap(), kept);
    fs::remove_file(fixture.root.join("fail")).unwrap();
    let result = success(&fixture.run(&[
        "bundle",
        "--target",
        "web",
        "--debug",
        "--base-path",
        "/override/",
        "--routing",
        "hash",
        "--json",
    ]));
    assert_eq!(result["profile"], "debug");
    let manifest: Value =
        serde_json::from_slice(&fs::read(output.join("bundle.json")).unwrap()).unwrap();
    assert_eq!(manifest["deployment"]["base_path"], "/override/");
    assert_eq!(manifest["deployment"]["routing"], "hash");
}
