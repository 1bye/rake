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
    time::{Duration, Instant},
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
        let control = self.directory.path().join("control");
        fs::create_dir_all(&control).unwrap();
        fs::copy(
            self.directory.path().join("app-double"),
            control.join("app"),
        )
        .unwrap();
        let events = fs::File::create(control.join("events")).unwrap();
        let errors = fs::File::create(control.join("errors")).unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_rake"))
            .current_dir(&self.root)
            .env("PATH", &self.tools)
            .env("FIXTURE", &self.root)
            .env("CONTROL", &control)
            .env("RAKE_CLI", self.tools.join("rake-double"))
            .args(["dev", "--json", "--no-open"])
            .args(extra)
            .stdout(events)
            .stderr(errors)
            .spawn()
            .unwrap();
        let mut running = Running {
            child,
            session: PathBuf::new(),
            control,
        };
        running.session = wait(|| {
            fs::read_to_string(running.control.join("events"))
                .ok()?
                .lines()
                .find_map(|line| {
                    let value: Value = serde_json::from_str(line).ok()?;
                    (value["event"] == "session")
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
        // SAFETY: the fixture owns this child, including during assertion unwinding.
        unsafe {
            libc::kill(self.child.id() as i32, libc::SIGTERM);
        }
        let _ = self.child.wait();
        if !self.session.as_os_str().is_empty() {
            let _ = fs::remove_dir_all(&self.session);
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
    let mut running = fixture.start(&["--restart-without-state", "--features", "theme"]);
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
fn blocks_restart_without_opt_in_and_owns_cdp() {
    let fixture = Fixture::new();
    let mut running = fixture.start(&["--cdp"]);
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
            .contains("--restart-without-state")
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
    let mut running = fixture.start(&[]);
    assert_eq!(running.phase("watching")["published"], 2);
    assert!(!running.session.join("builds/1").exists());
    fs::write(external.join("Cargo.toml"), "updated dependency").unwrap();
    wait(|| {
        let status = running.status();
        (status["phase"] == "reload_blocked" && status["build"] == 3).then_some(())
    });
    running.stop(libc::SIGTERM);
}
