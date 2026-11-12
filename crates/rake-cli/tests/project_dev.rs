//! Covers: native watch/publication, reload policy, private sessions, cancellation and CDP ownership.
//! Real: CLI supervisor, filesystem, IPC and Unix process groups.
//! Doubles: Cargo metadata, native bundler, console app and CDP adapter are shell tools.
//! Requires: Unix and /bin/sh; no Python, Node, GPU or network.

#![cfg(unix)]

use serde_json::{Value, json};

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command},
    thread,
    time::{Duration, Instant, SystemTime},
};

struct Fixture {
    directory: tempfile::TempDir,
    root: PathBuf,
    tools: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("app");
        let tools = directory.path().join("tools");

        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir(&tools).unwrap();
        fs::write(root.join("Cargo.toml"), "fixture").unwrap();
        fs::write(root.join("src/main.rs"), "first").unwrap();
        let package = json!({"id":"fixture", "name":"fixture", "version":"1.0.0", "source":null,
            "manifest_path":root.join("Cargo.toml"), "metadata":{}, "features":{},
            "dependencies":[{"name":"rake-app", "rename":"framework", "kind":null}],
            "targets":[{"name":"fixture", "kind":["bin"], "crate_types":["bin"]}]});

        fs::write(
            root.join("metadata.json"),
            json!({"workspace_root":root,
            "workspace_members":["fixture"], "packages":[package]})
            .to_string(),
        )
        .unwrap();
        let fixture = Self {
            directory,
            root,
            tools,
        };

        fixture.tool("cargo", "/bin/cat \"$FIXTURE/metadata.json\"");
        fixture.tool(
            "rake-double",
            r#"
if [ "$1" = cdp ]; then
    printf '%s' "$$" > "$CONTROL/cdp-pid"
    printf '{"event":"cdp_listening","url":"http://127.0.0.1:9999"}\n'
    exec /bin/sleep 300
fi
printf '%s\n' "$@" > "$CONTROL/build-args"
while [ "$1" != --output ]; do shift; done
output=$2
if [ -f "$FIXTURE/fail" ]; then exit 7; fi
if [ -f "$FIXTURE/hold" ]; then
    printf '%s' "$$" > "$CONTROL/build-pid"
    while [ -f "$FIXTURE/hold" ]; do /bin/sleep .05; done
fi
/bin/mkdir -p "$output/Fixture.app/Contents/MacOS"
/bin/cp "$CONTROL/app" "$output/Fixture.app/Contents/MacOS/fixture"
/bin/chmod 755 "$output/Fixture.app/Contents/MacOS/fixture"
if [ "$(/usr/bin/uname)" = Darwin ]; then
    printf '{"artifact":"%s/Fixture.app"}\n' "$output"
else
    printf '{"artifact":"%s/Fixture.app/Contents/MacOS/fixture"}\n' "$output"
fi
"#,
        );
        fixture.app("printf 'hello app\\n'\nprintf '\\036RAKE_DIAGNOSTIC {\"message\":\"structured\"}\\n' >&2\nexec /bin/sleep 300");

        fixture
    }

    fn tool(&self, name: &str, source: &str) {
        executable(&self.tools.join(name), source);
    }

    fn app(&self, source: &str) {
        executable(&self.directory.path().join("app-double"), source);
    }

    fn start(&self, extra: &[&str]) -> Running {
        self.start_options(extra, true)
    }

    fn start_options(&self, extra: &[&str], no_open: bool) -> Running {
        self.start_environment(extra, no_open, None)
    }

    fn start_environment(
        &self,
        extra: &[&str],
        no_open: bool,
        require_state: Option<&str>,
    ) -> Running {
        let control = self.directory.path().join("control");

        fs::create_dir_all(&control).unwrap();
        fs::copy(
            self.directory.path().join("app-double"),
            control.join("app"),
        )
        .unwrap();
        let events = fs::File::create(control.join("events")).unwrap();
        let errors = fs::File::create(control.join("errors")).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_rake"));

        command
            .current_dir(&self.root)
            .env("PATH", &self.tools)
            .env("FIXTURE", &self.root)
            .env("BROWSER", "fixture-browser --test %s")
            .env("CONTROL", &control)
            .env("RAKE_CLI", self.tools.join("rake-double"))
            .env("BLAZE_HOME", self.directory.path())
            .args(["dev", "--json"]);
        command.env_remove("RAKE_REQUIRE_STATE");

        if let Some(value) = require_state {
            command.env("RAKE_REQUIRE_STATE", value);
        }

        if no_open {
            command.arg("--no-open");
        }

        let started = SystemTime::now();

        let child = command
            .args(extra)
            .stdout(events)
            .stderr(errors)
            .spawn()
            .unwrap();

        let mut running = Running {
            started,
            child,
            session: PathBuf::new(),
            control,
        };

        running.session = wait(|| {
            // Web readiness can still be pending when the test cancels or kills the server.
            if let Ok(args) = fs::read_to_string(running.control.join("server-args")) {
                let mut args = args.lines();

                while let Some(argument) = args.next() {
                    if argument == "--development-state" {
                        return Some(PathBuf::from(args.next()?).parent()?.to_path_buf());
                    }
                }
            }

            fs::read_to_string(running.control.join("events"))
                .ok()?
                .lines()
                .find_map(|line| {
                    let value: Value = serde_json::from_str(line).ok()?;

                    value["session"]
                        .is_string()
                        .then(|| PathBuf::from(value["session"].as_str().unwrap()))
                })
        });

        running
    }
}

fn executable(path: &Path, source: &str) {
    fs::write(path, format!("#!/bin/sh\n{source}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

struct Running {
    started: SystemTime,
    child: Child,
    session: PathBuf,
    control: PathBuf,
}

impl Running {
    fn status(&self) -> Value {
        serde_json::from_slice(&fs::read(self.session.join("status.json")).unwrap()).unwrap()
    }

    fn phase(&self, phase: &str) -> Value {
        wait(|| {
            let value = self.status();

            (value["phase"] == phase).then_some(value)
        })
    }

    fn stop(&mut self, signal: i32) {
        // SAFETY: signal only the child created by this fixture.
        unsafe {
            libc::kill(self.child.id() as i32, signal);
        }
        wait(|| self.child.try_wait().unwrap());
        assert!(self.child.wait().unwrap().success());
        assert_eq!(self.status()["phase"], "stopped");
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!(
                "supervisor fixture errors: {}",
                fs::read_to_string(self.control.join("errors")).unwrap_or_default()
            );

            if let Ok(status) = fs::read_to_string(self.session.join("status.json")) {
                eprintln!("supervisor fixture status: {status}");
            }
        }

        // SAFETY: the fixture owns this child, including during assertion unwinding.
        unsafe {
            libc::kill(self.child.id() as i32, libc::SIGTERM);
        }
        let _ = self.child.wait();

        if !self.session.as_os_str().is_empty() {
            let _ = fs::remove_dir_all(&self.session);
        } else {
            remove_early_session(self.child.id(), self.started);
        }
    }
}

// Web startup can fail before the listening event publishes its session path.
fn remove_early_session(pid: u32, started: SystemTime) {
    for entry in fs::read_dir("/tmp").unwrap().flatten() {
        if !entry.file_name().to_string_lossy().starts_with("rake-dev-") {
            continue;
        }

        let marker = entry.path().join("status.json");

        if !marker
            .metadata()
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| modified >= started)
        {
            continue;
        }

        let value = fs::read(&marker)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());

        if value.is_some_and(|value| value["pid"] == pid) {
            fs::remove_dir_all(entry.path()).unwrap();
        }
    }
}

fn wait<T>(mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(15);

    loop {
        if let Some(value) = probe() {
            return value;
        }

        assert!(Instant::now() < deadline, "condition timed out");
        thread::sleep(Duration::from_millis(25));
    }
}

fn alive(pid: u32) -> bool {
    // SAFETY: signal zero tests existence without mutating the process.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[test]
fn watches_publishes_keeps_old_app_on_failure_restarts_and_survives_crash() {
    let fixture = Fixture::new();
    let mut running = fixture.start(&["--features", "theme"]);
    let first = running.phase("watching");
    let pid = first["runtime_pid"].as_u64().unwrap() as u32;

    assert!(alive(pid));
    assert_eq!(first["mechanism"], "native_restart");
    // Publication does not imply the newly spawned app has emitted its first line.
    wait(|| {
        let bytes = fs::read(running.session.join("output.json")).ok()?;
        let output: Value = serde_json::from_slice(&bytes).ok()?;

        output["records"]
            .as_array()?
            .iter()
            .any(|record| record["structured"] == true)
            .then_some(())
    });
    assert!(
        fs::read_to_string(running.control.join("build-args"))
            .unwrap()
            .contains("theme,framework/devtools")
    );
    fs::write(fixture.root.join("fail"), "fail").unwrap();
    let failed = running.phase("build_failed");

    assert_eq!(failed["published"], first["published"]);
    assert!(alive(pid));
    assert!(!running.session.join("builds/2").exists());
    fs::remove_file(fixture.root.join("fail")).unwrap();
    fs::write(fixture.root.join("src/main.rs"), "second").unwrap();
    let second = wait(|| {
        let s = running.status();

        (s["phase"] == "watching" && s["published"].as_u64()? > 1).then_some(s)
    });

    let next_pid = second["runtime_pid"].as_u64().unwrap() as u32;

    assert_ne!(next_pid, pid);
    assert!(!alive(pid));
    // SAFETY: simulate a crash of the app launched by our fixture.
    unsafe {
        libc::kill(next_pid as i32, libc::SIGKILL);
    }
    assert_eq!(running.phase("app_exited")["exit_code"], -9);
    fs::write(fixture.root.join("src/main.rs"), "third").unwrap();
    let third = wait(|| {
        let s = running.status();

        (s["phase"] == "watching" && s["published"].as_u64()? > second["published"].as_u64()?)
            .then_some(s)
    });

    let status = Command::new(env!("CARGO_BIN_EXE_rake"))
        .args(["dev", "status", "--session"])
        .arg(&running.session)
        .arg("--json")
        .env("PATH", &fixture.tools)
        .output()
        .unwrap();

    assert!(status.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&status.stdout).unwrap(),
        third
    );
    running.stop(libc::SIGINT);
    assert!(!alive(third["runtime_pid"].as_u64().unwrap() as u32));
    assert_eq!(
        fs::read_dir(running.session.join("control"))
            .unwrap()
            .count(),
        0
    );

    for path in [&running.session, &running.session.join("control")] {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    for name in ["status.json", "output.json"] {
        assert_eq!(
            fs::metadata(running.session.join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    let output: Value =
        serde_json::from_slice(&fs::read(running.session.join("output.json")).unwrap()).unwrap();

    assert!(
        output["records"]
            .as_array()
            .unwrap()
            .iter()
            .any(|record| record["structured"] == true),
        "{output}"
    );
    assert!(
        output["records"]
            .as_array()
            .unwrap()
            .iter()
            .any(|record| record["stream"] == "lifecycle" && record["level"] == "error")
    );
}

#[test]
fn strict_mode_blocks_restart_without_endpoint_and_owns_cdp() {
    let fixture = Fixture::new();
    let mut running = fixture.start(&["--cdp", "--require-state"]);
    let first = running.phase("watching");

    let cdp_pid: u32 = fs::read_to_string(running.control.join("cdp-pid"))
        .unwrap()
        .parse()
        .unwrap();

    assert!(alive(cdp_pid));
    let cdp: Value =
        serde_json::from_slice(&fs::read(running.session.join("cdp.json")).unwrap()).unwrap();

    assert_eq!(cdp["url"], "http://127.0.0.1:9999");
    fs::write(fixture.root.join("src/main.rs"), "second").unwrap();
    assert!(
        running.phase("reload_blocked")["error"]
            .as_str()
            .unwrap()
            .contains("--require-state")
    );
    assert!(alive(first["runtime_pid"].as_u64().unwrap() as u32));
    running.stop(libc::SIGTERM);
    assert!(!alive(cdp_pid));
}

#[test]
fn cancellation_during_build_cleans_compiler_group_and_stops_session() {
    let fixture = Fixture::new();

    fs::write(fixture.root.join("hold"), "hold").unwrap();
    let mut running = fixture.start(&[]);

    let pid: u32 = wait(|| {
        fs::read_to_string(running.control.join("build-pid"))
            .ok()?
            .parse()
            .ok()
    });

    running.stop(libc::SIGTERM);
    assert!(!alive(pid));
    assert_eq!(running.status()["published"], 0);
}

#[test]
fn superseded_build_is_discarded_before_publication() {
    let fixture = Fixture::new();

    fs::write(fixture.root.join("hold"), "hold").unwrap();
    let mut running = fixture.start(&[]);

    wait(|| fs::read_to_string(running.control.join("build-pid")).ok());
    fs::write(fixture.root.join("src/main.rs"), "changed during build").unwrap();
    fs::remove_file(fixture.root.join("hold")).unwrap();
    let status = running.phase("watching");

    assert_eq!(status["published"], 2);
    assert!(!running.session.join("builds/1").exists());
    assert!(
        fs::read_to_string(running.control.join("events"))
            .unwrap()
            .contains("build_superseded")
    );
    running.stop(libc::SIGINT);
}

#[test]
fn cleanup_kills_descendants_that_ignore_sigterm() {
    let fixture = Fixture::new();

    fixture.app("/bin/sh -c 'trap \"\" TERM; echo $$ > \"$CONTROL/descendant\"; exec /bin/sleep 300' &\nwait");
    let mut running = fixture.start(&[]);

    running.phase("watching");
    let descendant: u32 = wait(|| {
        fs::read_to_string(running.control.join("descendant"))
            .ok()?
            .trim()
            .parse()
            .ok()
    });

    running.stop(libc::SIGTERM);
    wait(|| {
        let status = Command::new("/bin/ps")
            .args(["-o", "stat=", "-p", &descendant.to_string()])
            .output()
            .unwrap();

        (!alive(descendant)
            || String::from_utf8_lossy(&status.stdout)
                .trim()
                .starts_with('Z'))
        .then_some(())
    });
}

#[test]
fn cdp_startup_failure_keeps_exit_one_and_cancellable_readiness_stops_cleanly() {
    for ready in [true, false] {
        let fixture = Fixture::new();

        fixture.tool(
            "rake-double",
            if ready {
                "printf '{\"event\":\"wrong\"}\\n'\nexec /bin/sleep 300"
            } else {
                "printf '%s' \"$$\" > \"$CONTROL/cdp-pid\"\nexec /bin/sleep 300"
            },
        );
        let mut running = fixture.start(&["--cdp"]);

        if ready {
            let status = wait(|| running.child.try_wait().unwrap());

            assert_eq!(status.code(), Some(1));
            assert_eq!(running.status()["phase"], "stopped");
            assert!(
                fs::read_to_string(running.control.join("errors"))
                    .unwrap()
                    .contains("CDP adapter could not start")
            );
        } else {
            let pid: u32 = wait(|| {
                fs::read_to_string(running.control.join("cdp-pid"))
                    .ok()?
                    .parse()
                    .ok()
            });

            running.stop(libc::SIGTERM);
            assert!(!alive(pid));
        }
    }
}

#[test]
fn metadata_refresh_includes_new_local_watch_roots_before_publication() {
    let fixture = Fixture::new();
    let external = fixture.directory.path().join("local-dependency");

    fs::create_dir(&external).unwrap();
    fs::write(external.join("Cargo.toml"), "local manifest").unwrap();
    let mut metadata: Value =
        serde_json::from_slice(&fs::read(fixture.root.join("metadata.json")).unwrap()).unwrap();

    metadata["packages"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"local", "source":null, "manifest_path":external.join("Cargo.toml")}));
    fs::write(
        fixture.directory.path().join("extended-metadata.json"),
        metadata.to_string(),
    )
    .unwrap();
    fixture.tool("cargo", r#"
count=0
if [ -f "$CONTROL/metadata-count" ]; then count=$(/bin/cat "$CONTROL/metadata-count"); fi
count=$((count + 1))
printf '%s' "$count" > "$CONTROL/metadata-count"
if [ "$count" -gt 2 ]; then /bin/cat "$FIXTURE/../extended-metadata.json"; else /bin/cat "$FIXTURE/metadata.json"; fi
"#);
    // Keep the blocked-publication assertion under the opt-in strict policy.
    let mut running = fixture.start(&["--require-state"]);

    assert_eq!(running.phase("watching")["published"], 2);
    assert!(!running.session.join("builds/1").exists());
    fs::write(external.join("Cargo.toml"), "updated dependency").unwrap();
    wait(|| {
        let status = running.status();

        (status["phase"] == "reload_blocked" && status["build"] == 3).then_some(())
    });
    running.stop(libc::SIGTERM);
}

impl Fixture {
    fn web(&self) {
        let mut metadata: Value =
            serde_json::from_slice(&fs::read(self.root.join("metadata.json")).unwrap()).unwrap();

        metadata["packages"][0]["targets"] = json!([
            {"name":"fixture", "kind":["cdylib"], "crate_types":["cdylib"]}
        ]);
        fs::write(self.root.join("metadata.json"), metadata.to_string()).unwrap();
        fs::write(
            self.root.join("Cargo.lock"),
            "[[package]]\nname='wasm-bindgen'\nversion='0.2.100'\n",
        )
        .unwrap();
        fs::write(self.root.join("input.wasm"), "wasm fixture").unwrap();
        self.tool("cargo", r#"
if [ "$1" = metadata ]; then /bin/cat "$FIXTURE/metadata.json"; exit; fi
printf '%s\n' "$@" > "$CONTROL/build-args"
printf 'web build diagnostic\n' >&2
if [ -f "$FIXTURE/fail" ]; then exit 7; fi
if [ -f "$FIXTURE/hold" ]; then
    printf '%s' "$$" > "$CONTROL/build-pid"
    while [ -f "$FIXTURE/hold" ]; do /bin/sleep .05; done
fi
printf '{"reason":"compiler-artifact","package_id":"fixture","target":{"name":"fixture"},"filenames":["%s/input.wasm"]}\n' "$FIXTURE"
"#);
        self.tool(
            "wasm-bindgen",
            r#"
if [ "$1" = --version ]; then printf 'wasm-bindgen 0.2.100\n'; exit; fi
/bin/mkdir -p "$4"
/bin/cp "$5" "$4/fixture_bg.wasm"
printf 'export default async function init() {}' > "$4/fixture.js"
"#,
        );
        self.tool(
            "python3",
            "printf forbidden > \"$CONTROL/python-called\"; exit 99",
        );
        self.tool(
            "fixture-browser",
            "printf '%s\\n' \"$@\" >> \"$CONTROL/browser-args\"",
        );
        self.tool("node", r#"
if [ "$1" = --version ]; then printf 'v26.0.0\n'; exit; fi
printf '%s' "$$" > "$CONTROL/node-pid"
printf '%s\n' "$@" >> "$CONTROL/node-args"
if [ -f "$CONTROL/node-bad" ]; then printf '{"event":"wrong"}\n'; else
printf '{"event":"embedded_devtools_listening","url":"http://127.0.0.1:9876/embedded-devtools-fixture.html"}\n'; fi
exec /bin/sleep 300
"#);
    }
}

#[cfg(target_os = "macos")]
impl Fixture {
    fn preview(&self) {
        self.web();
        // Retain the desktop package/metadata and native build adapter.
        let desktop = Fixture::new();

        fs::copy(
            desktop.root.join("metadata.json"),
            self.root.join("metadata.json"),
        )
        .unwrap();
        let mut metadata: Value =
            serde_json::from_slice(&fs::read(self.root.join("metadata.json")).unwrap()).unwrap();

        metadata["workspace_root"] = json!(self.root);
        metadata["packages"][0]["manifest_path"] = json!(self.root.join("Cargo.toml"));
        fs::write(self.root.join("metadata.json"), metadata.to_string()).unwrap();
        self.tool("cargo", "/bin/cat \"$FIXTURE/metadata.json\"");
        self.tool(
            "xcrun",
            r#"
printf '%s\n' "$@" > "$CONTROL/swift-args"
while [ "$1" != -o ]; do shift; done
printf 'capture fixture' > "$2"
"#,
        );
        self.app(
            "printf '%s' \"$RAKE_DEV_PREVIEW\" > \"$CONTROL/preview-env\"\nexec /bin/sleep 300",
        );
    }
}

#[test]
fn web_publication_owns_server_and_single_companion_and_retains_old_build_on_failure() {
    let fixture = Fixture::new();

    fixture.web();
    let mut running = fixture.start(&["--target=web", "--port=0", "--cdp"]);

    wait(|| running.status()["devtools_url"].as_str().map(str::to_owned));
    let first = running.phase("watching");

    assert_eq!(first["target"], "web");
    assert_eq!(first["mechanism"], "web_page_reload");
    assert_eq!(first["published"], 1);
    assert!(
        !running.control.join("server-pid").exists(),
        "Web serving must not launch Python"
    );
    let node_pid: u32 = fs::read_to_string(running.control.join("node-pid"))
        .unwrap()
        .parse()
        .unwrap();

    let cdp_pid: u32 = fs::read_to_string(running.control.join("cdp-pid"))
        .unwrap()
        .parse()
        .unwrap();

    assert!(alive(node_pid) && alive(cdp_pid));
    let events = fs::read_to_string(running.control.join("events")).unwrap();

    let ready: Value = events
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|value| value["event"] == "web_listening")
        .unwrap();

    assert!(
        fs::read_to_string(running.control.join("node-args"))
            .unwrap()
            .contains(ready["url"].as_str().unwrap())
    );
    fs::write(fixture.root.join("fail"), "fail").unwrap();
    assert_eq!(running.phase("build_failed")["published"], 1);
    assert!(
        Path::new(first["directory"].as_str().unwrap())
            .join("index.html")
            .is_file()
    );
    fs::remove_file(fixture.root.join("fail")).unwrap();
    wait(|| {
        let status = running.status();

        (status["phase"] == "watching" && status["published"].as_u64()? > 1).then_some(())
    });
    assert_eq!(
        fs::read_to_string(running.control.join("node-args"))
            .unwrap()
            .lines()
            .filter(|line| line.ends_with("embedded-devtools.mjs"))
            .count(),
        1
    );
    let node_args = fs::read_to_string(running.control.join("node-args")).unwrap();

    let helpers = PathBuf::from(node_args.lines().next().unwrap())
        .parent()
        .unwrap()
        .to_path_buf();

    running.stop(libc::SIGTERM);
    assert!(!alive(node_pid) && !alive(cdp_pid));
    assert!(
        !helpers.exists(),
        "extracted helpers must have session ownership"
    );
    let output: Value =
        serde_json::from_slice(&fs::read(running.session.join("output.json")).unwrap()).unwrap();

    assert!(
        output["records"]
            .as_array()
            .unwrap()
            .iter()
            .any(|record| record["stream"] == "build"
                && record["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("web build diagnostic"))),
        "{output}"
    );
}

#[test]
#[cfg(target_os = "macos")]
fn preview_listener_shutdown_cancels_partial_readers_without_python() {
    use std::io::{Read, Write};
    let fixture = Fixture::new();

    fixture.preview();
    fs::remove_file(fixture.tools.join("python3")).unwrap();
    let mut running = fixture.start(&["--preview", "--port", "0"]);

    running.phase("watching");
    assert!(!running.control.join("server-pid").exists());
    let event: Value = fs::read_to_string(running.control.join("events"))
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|event| event["event"] == "preview_listening")
        .unwrap();

    let url = url::Url::parse(event["url"].as_str().unwrap()).unwrap();
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", url.port().unwrap())).unwrap();

    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    write!(stream, "GET /__rake/preview HTTP/1.1\r\n").unwrap();
    running.stop(libc::SIGTERM);
    let reply = stream.read(&mut [0]);

    assert!(
        matches!(reply, Ok(0))
            || reply.is_err_and(|error| matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
            ))
    );
}

#[test]
#[cfg(target_os = "macos")]
fn preview_compiler_failure_is_fatal_and_no_cdp_needs_no_node() {
    for bad in [false, true] {
        let fixture = Fixture::new();

        fixture.preview();
        fs::remove_file(fixture.tools.join("node")).unwrap();

        if bad {
            fixture.tool("xcrun", "exit 7");
        }

        let mut running = fixture.start(&["--preview", "--port", "0"]);

        if bad {
            assert_eq!(wait(|| running.child.try_wait().unwrap()).code(), Some(1));
            assert!(
                fs::read_to_string(running.control.join("errors"))
                    .unwrap()
                    .contains("xcrun exited 7")
            );
        } else {
            assert_eq!(running.phase("watching")["published"], 1);
            assert!(!running.control.join("cdp-pid").exists());
            running.stop(libc::SIGTERM);
        }
    }
}

#[test]
fn web_cdp_rejects_missing_old_invalid_or_failing_node_before_loading_project() {
    let fixture = Fixture::new();

    for source in [
        None,
        Some("printf 'v21.9.0\\n'"),
        Some("printf 'invalid\\n'"),
        Some("printf 'v26.0.0\\n'; exit 7"),
    ] {
        if let Some(source) = source {
            fixture.tool("node", source);
        } else {
            let _ = fs::remove_file(fixture.tools.join("node"));
        }

        let result = Command::new(env!("CARGO_BIN_EXE_rake"))
            .env("PATH", &fixture.tools)
            .current_dir(fixture.directory.path())
            .args(["dev", "--target=web", "--cdp", "--json"])
            .output()
            .unwrap();

        assert_eq!(result.status.code(), Some(2), "{result:?}");
        let error: Value = serde_json::from_slice(&result.stderr).unwrap();

        assert_eq!(
            error["error"]["message"],
            "Web Chrome DevTools requires Node 22 or later"
        );
    }
}

#[test]
fn browser_opening_honors_browser_and_cdp_opens_only_the_companion() {
    for cdp in [false, true] {
        let fixture = Fixture::new();

        fixture.web();
        let args = if cdp {
            vec!["--target=web", "--port=0", "--cdp"]
        } else {
            vec!["--target=web", "--port=0"]
        };

        let mut running = fixture.start_options(&args, false);

        running.phase("watching");
        let args = wait(|| fs::read_to_string(running.control.join("browser-args")).ok());

        assert_eq!(args.lines().take(1).collect::<Vec<_>>(), ["--test"]);
        assert_eq!(args.lines().count(), 2, "open exactly one browser URL");
        assert_eq!(args.contains("embedded-devtools-fixture.html"), cdp);
        running.stop(libc::SIGTERM);
    }
}

#[test]
fn web_companion_exit_is_fatal_and_invalid_readiness_blocks_publication() {
    for bad in [false, true] {
        let fixture = Fixture::new();

        fixture.web();
        fs::create_dir(fixture.directory.path().join("control")).unwrap();

        if bad {
            fs::write(fixture.directory.path().join("control/node-bad"), "bad").unwrap();
        }

        let mut running = fixture.start(&["--target", "web", "--port", "0", "--cdp"]);

        let pid: u32 = wait(|| {
            fs::read_to_string(running.control.join("node-pid"))
                .ok()?
                .parse()
                .ok()
        });

        if bad {
            assert!(
                running.phase("reload_blocked")["error"]
                    .as_str()
                    .unwrap()
                    .contains("embedded Chrome DevTools could not start")
            );
            running.stop(libc::SIGTERM);
        } else {
            wait(|| running.status()["devtools_url"].as_str().map(str::to_owned));
            // SAFETY: this fixture owns the service being killed.
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
            assert_eq!(wait(|| running.child.try_wait().unwrap()).code(), Some(1));
            assert!(
                fs::read_to_string(running.control.join("errors"))
                    .unwrap()
                    .contains("embedded Chrome DevTools companion exited")
            );
        }

        assert!(!alive(pid));
    }
}

#[test]
#[cfg(target_os = "macos")]
fn preview_session_compiles_helper_before_server_and_marks_app_environment() {
    let fixture = Fixture::new();

    fixture.preview();
    let mut running = fixture.start(&["--preview", "--cdp", "--port", "0"]);

    running.phase("watching");
    assert_eq!(
        wait(|| fs::read_to_string(running.control.join("preview-env")).ok()),
        "1"
    );
    assert!(!running.control.join("server-pid").exists());
    assert!(running.session.join("preview-capture").is_file());
    assert!(
        fs::read_to_string(running.control.join("swift-args"))
            .unwrap()
            .contains("-module-cache-path")
    );
    assert!(!running.control.join("node-pid").exists());
    running.stop(libc::SIGTERM);
}

#[test]
fn web_serves_without_python_or_node_and_reports_occupied_listener() {
    let fixture = Fixture::new();

    fixture.web();
    fs::remove_file(fixture.tools.join("python3")).unwrap();
    fs::remove_file(fixture.tools.join("node")).unwrap();
    let mut running = fixture.start(&["--target=web", "--port=0"]);

    assert_eq!(running.phase("watching")["published"], 1);
    assert!(!running.control.join("server-pid").exists());
    let events = fs::read_to_string(running.control.join("events")).unwrap();

    let ready: Value = events
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|value| value["event"] == "web_listening")
        .unwrap();

    let url = url::Url::parse(ready["url"].as_str().unwrap()).unwrap();
    let address = format!("127.0.0.1:{}", url.port().unwrap());
    let mut stream = std::net::TcpStream::connect(&address).unwrap();

    use std::io::{Read, Write};
    write!(stream, "GET / HTTP/1.0\r\nHost: {address}\r\n\r\n").unwrap();
    let mut response = String::new();

    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.0 200"), "{response}");
    assert!(response.contains("/__rake/bootstrap.js?build=1"));
    running.stop(libc::SIGTERM);
    assert!(std::net::TcpStream::connect(&address).is_err());

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    let started = SystemTime::now();

    let child = Command::new(env!("CARGO_BIN_EXE_rake"))
        .current_dir(&fixture.root)
        .env("PATH", &fixture.tools)
        .env("FIXTURE", &fixture.root)
        .args([
            "dev",
            "--target=web",
            "--port",
            &port,
            "--no-open",
            "--json",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    let pid = child.id();
    let result = child.wait_with_output().unwrap();

    remove_early_session(pid, started);
    assert_eq!(result.status.code(), Some(1));
    let error: Value = serde_json::from_slice(&result.stderr).unwrap();

    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Address already in use")
    );
}

#[test]
fn blaze_direct_edits_fall_back_for_structured_errors_and_keep_the_old_process() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    let fixture = Fixture::new();

    fixture.tool("blaze",r#"
printf '%s\n' "$@" > "$CONTROL/blaze-args"
printf '%s,%s,%s,%s\n' "$BLAZE_PROC_MACRO_WASM" "$BLAZE_PROC_MACRO_SERVER" "$BLAZE_SANDBOX" "$LIBLZMA_NO_PKG_CONFIG" > "$CONTROL/blaze-env"
printf 'build\n' >> "$CONTROL/blaze-builds"
if [ -f "$CONTROL/fail-blaze" ]; then
    printf '{"event":{"Finished":{"ok":false,"summary":{"error":"error[E0425]: missing helper"}}}}\n'
    exit 1
fi
/bin/mkdir -p "$FIXTURE/target/debug" "$FIXTURE/target/blaze/whole/worker"
/bin/cp "$CONTROL/app" "$FIXTURE/target/debug/fixture"
/bin/cp "$CONTROL/app" "$FIXTURE/target/blaze/whole/worker/fixture"
"#);
    let listener = UnixListener::bind(fixture.directory.path().join("daemon.sock")).unwrap();

    listener.set_nonblocking(true).unwrap();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stopping = stop.clone();
    let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = requests.clone();

    let daemon = thread::spawn(move || {
        while !stopping.load(std::sync::atomic::Ordering::Acquire) {
            match listener.accept() {
                Ok((mut connection, _)) => {
                    connection
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut line = String::new();

                    BufReader::new(&connection).read_line(&mut line).unwrap();
                    let mut requests = seen.lock().unwrap();

                    requests.push(line);
                    let reply = if requests.len() == 2 {
                        "error\tcompile failed\n"
                    } else {
                        "ok\n"
                    };

                    let _ = connection.write_all(reply.as_bytes());
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5))
                }
                Err(error) => panic!("fake daemon failed: {error}"),
            }
        }
    });

    let mut running = fixture.start(&["--compiler", "blaze"]);
    let first = running.phase("watching");
    let pid = first["runtime_pid"].as_u64().unwrap();

    assert_eq!(
        fs::read_to_string(running.control.join("blaze-env"))
            .unwrap()
            .trim(),
        "0,0,on,1"
    );
    let arguments = fs::read_to_string(running.control.join("blaze-args")).unwrap();

    assert!(arguments.contains("--bin\nfixture"));
    assert!(arguments.contains("framework/devtools,framework/live"));
    fs::write(fixture.root.join("src/main.rs"), "direct source\n").unwrap();
    wait(|| {
        let s = running.status();

        (s["phase"] == "watching"
            && s["build"].as_u64().unwrap() > first["build"].as_u64().unwrap())
        .then_some(())
    });
    assert_eq!(running.status()["runtime_pid"], pid);
    assert_eq!(
        fs::read_to_string(running.control.join("blaze-builds"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    fs::write(running.control.join("fail-blaze"), "fail").unwrap();
    fs::write(fixture.root.join("src/main.rs"), "compile error").unwrap();
    let failed = running.phase("build_failed");

    assert!(
        failed["error"]
            .as_str()
            .unwrap()
            .contains("error[E0425]: missing helper"),
        "{failed}"
    );
    assert_eq!(failed["runtime_pid"], pid);
    fs::remove_file(running.control.join("fail-blaze")).unwrap();
    fs::write(fixture.root.join("src/main.rs"), "fixed source").unwrap();
    running.phase("watching");
    assert_eq!(running.status()["runtime_pid"], pid);
    assert_eq!(
        fs::read_to_string(running.control.join("blaze-builds"))
            .unwrap()
            .lines()
            .count(),
        2
    );
    running.stop(libc::SIGTERM);
    stop.store(true, std::sync::atomic::Ordering::Release);
    daemon.join().unwrap();
    let requests = requests.lock().unwrap();

    assert_eq!(requests.len(), 3);
    assert!(requests[0].ends_with("direct source\\n\n"));
}

#[test]
fn require_state_environment_blocks_then_accepts_a_later_edit() {
    let fixture = Fixture::new();
    let mut running = fixture.start_environment(&[], true, Some("1"));
    let pid = running.phase("watching")["runtime_pid"].as_u64().unwrap() as u32;

    fs::write(fixture.root.join("src/main.rs"), "second").unwrap();
    let first = running.phase("reload_blocked");

    assert!(first["error"].as_str().unwrap().contains("--require-state"));
    assert!(alive(pid));
    fs::write(fixture.root.join("src/main.rs"), "third").unwrap();
    let next = wait(|| {
        let status = running.status();

        (status["phase"] == "reload_blocked"
            && status["build"].as_u64()? > first["build"].as_u64()?)
        .then_some(status)
    });

    assert_eq!(next["runtime_pid"], pid);
    running.stop(libc::SIGTERM);
}
