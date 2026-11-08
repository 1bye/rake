//! Covers: fingerprint exclusions, keyed retry, reload preparation/abort/restore and output bounds.
//! Real: filesystem, private JSON, Unix sockets and process groups.
//! Doubles: a runtime speaks the existing JSON protocol; a shell backend replaces code.
//! Requires: Unix and /bin/sh; no Python, GPU, Node or network.

use super::*;
use std::{
    io::{BufRead, BufReader},
    os::unix::{
        fs::{PermissionsExt, symlink},
        net::UnixListener,
    },
    sync::{Arc, Mutex},
};

fn project(root: &Path) -> Project {
    Project {
        package: json!({"name":"fixture", "features":{}, "dependencies":[]}),
        root: root.to_owned(),
        target: "fixture".into(),
        watch_roots: vec![root.to_owned(), root.join("src")],
        web_dir: root.join("web"),
        assets: None,
        bindgen: None,
    }
}

#[test]
fn fingerprint_hashes_contents_in_python_order_ignores_outputs_links_and_deduplicates() {
    use sha2::{Digest, Sha256};
    use std::os::unix::ffi::OsStrExt;
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    fs::create_dir(root.join("src")).unwrap();
    fs::create_dir(root.join("target")).unwrap();
    fs::create_dir(root.join("output")).unwrap();
    fs::write(root.join("Cargo.toml"), "manifest").unwrap();
    fs::write(root.join("src/main.rs"), "first").unwrap();
    fs::write(root.join("target/ignored"), "ignored").unwrap();
    symlink(root.join("src"), root.join("alias")).unwrap();
    let project = project(root);
    let output = root.join("output");
    let first = fingerprint(&project, Some(&output)).unwrap();
    let mut expected = Sha256::new();
    for (name, contents) in [("Cargo.toml", "manifest"), ("src/main.rs", "first")] {
        expected.update(root.join(name).as_os_str().as_bytes());
        expected.update(contents);
    }
    assert_eq!(first, <[u8; 32]>::from(expected.finalize()));
    fs::write(output.join("ignored"), "changed").unwrap();
    fs::write(root.join("target/ignored"), "changed").unwrap();
    assert_eq!(first, fingerprint(&project, Some(&output)).unwrap());
    fs::write(root.join("src/main.rs"), "other").unwrap();
    assert_ne!(first, fingerprint(&project, Some(&output)).unwrap());
    fs::remove_file(root.join("src/main.rs")).unwrap();
    assert_ne!(first, fingerprint(&project, Some(&output)).unwrap());
}

#[test]
fn process_output_matches_the_private_ring_and_utf8_byte_bounds() {
    let output = ProcessOutput::new();
    let mut output = output.lock().unwrap();
    for _ in 0..130 {
        output.append(&"漢".repeat(1000), 123, "stdout", None, false);
    }
    let snapshot = output.snapshot();
    let records = snapshot["records"].as_array().unwrap();
    assert_eq!(records.len(), 128);
    assert_eq!(snapshot["dropped"], 2);
    assert_eq!(snapshot["revision"], 130);
    assert_eq!(records[0]["sequence"], 3);
    assert_eq!(records[0]["text"].as_str().unwrap().len(), 2046);
    assert_eq!(records[0]["truncated"], true);
    assert!(records[0]["level"].is_null());
    assert!(records[0]["elapsed_us"].is_u64());
}

struct Runtime {
    directory: tempfile::TempDir,
    instance: Value,
    calls: Arc<Mutex<Vec<Value>>>,
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Runtime {
    fn new(reply: impl Fn(&Value, usize) -> Option<Value> + Send + 'static) -> Self {
        let directory = tempfile::Builder::new()
            .prefix("rake-ipc-")
            .tempdir_in("/tmp")
            .unwrap();
        let path = directory.path().join("runtime.sock");
        let listener = UnixListener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stopping = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let stop = stopping.clone();
        let seen = calls.clone();
        let thread = thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut connection, _)) => {
                        connection
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let mut line = String::new();
                        BufReader::new(&connection).read_line(&mut line).unwrap();
                        let request: Value = serde_json::from_str(&line).unwrap();
                        let index = {
                            let mut calls = seen.lock().unwrap();
                            calls.push(request.clone());
                            calls.len()
                        };
                        if let Some(response) = reply(&request["message"]["value"], index) {
                            let _ = writeln!(connection, "{}", json!({"Ok":{"value":response}}));
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("runtime accept failed: {error}"),
                }
            }
        });
        Self {
            directory,
            instance: json!({"application_id":"fixture", "info":{"runtime_id":"fixture-runtime"}, "endpoint":{"path":path}}),
            calls,
            stopping,
            thread: Some(thread),
        }
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        self.thread.take().unwrap().join().unwrap();
    }
}

#[test]
fn native_transport_retry_keeps_the_same_request_identity() {
    let runtime = Runtime::new(|_, index| (index > 1).then(|| json!({"phase":"ready"})));
    let result = exchange(
        &runtime.instance,
        "command",
        json!({"type":"prepare_reload"}),
        Duration::from_secs(1),
        None,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(result["phase"], "ready");
    let calls = runtime.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0], calls[1]);
    assert!(calls[0]["key"].is_string());
}

fn replies(value: &Value, _: usize) -> Option<Value> {
    Some(match value["type"].as_str().unwrap() {
        "reload" => json!({"phase":"ready"}),
        "operations" => json!([]),
        "checkpoint" => json!({"generation":7,"draft":"é漢字"}),
        _ => Value::Null,
    })
}

#[test]
fn native_backend_failure_aborts_and_success_applies_without_stopping_old_host() {
    for success in [false, true] {
        let runtime = Runtime::new(replies);
        let root = runtime.directory.path();
        let backend = root.join("backend");
        fs::write(&backend, format!("#!/bin/sh\nprintf '%s' \"$RAKE_RELOAD_GENERATION\" > \"$RAKE_RELOAD_PROJECT/generation\"\nprintf 'backend diagnostic\\n'\nexit {}\n", if success {0} else {1})).unwrap();
        fs::set_permissions(&backend, fs::Permissions::from_mode(0o700)).unwrap();
        let args = Args {
            dev: true,
            json: true,
            native_reloader: Some(backend),
            ..Args::default()
        };
        let cancelled = AtomicBool::new(false);
        let session = root.join("session");
        fs::create_dir(&session).unwrap();
        let mut supervisor = Supervisor::new(&args, project(root), session, &cancelled).unwrap();
        supervisor.app = Some(
            Processes::new(&cancelled)
                .start(Command::new("/bin/sleep").arg("300"))
                .unwrap(),
        );
        let pid = supervisor.app.as_ref().unwrap().0.id();
        let mut instance = runtime.instance.clone();
        instance["pid"] = json!(pid);
        write_marker(&supervisor.control.join("runtime.json"), &instance).unwrap();
        let result = supervisor.publish(2, &root.join("artifact"));
        assert_eq!(result.is_ok(), success);
        assert!(
            supervisor
                .app
                .as_mut()
                .unwrap()
                .0
                .try_wait()
                .unwrap()
                .is_none()
        );
        assert_eq!(fs::read_to_string(root.join("generation")).unwrap(), "8");
        let calls = runtime.calls.lock().unwrap();
        let prepare = calls
            .iter()
            .find(|call| call["message"]["value"]["type"] == "prepare_reload")
            .unwrap();
        let finish = calls.last().unwrap();
        assert_eq!(finish["message"]["value"]["applied"], success);
        assert_eq!(
            finish["message"]["value"]["id"],
            prepare["message"]["value"]["request"]["id"]
        );
        assert!(finish["key"].as_str().unwrap().ends_with(if success {
            "/apply"
        } else {
            "/abort"
        }));
        assert_eq!(supervisor.checkpoint.exists(), !success);
        if success {
            assert_eq!(supervisor.status["published"], 2);
        }
        supervisor.finish().unwrap();
    }
}

#[test]
fn stateful_restart_prepares_checkpoint_and_busy_operations_delay_publication() {
    let busy = Arc::new(AtomicBool::new(true));
    let state = busy.clone();
    let runtime = Runtime::new(move |value, index| {
        if value["type"] == "operations" && state.load(Ordering::Acquire) {
            Some(json!([["operation", {"AwaitingDecision":"question"}]]))
        } else {
            replies(value, index)
        }
    });
    let root = runtime.directory.path();
    let artifact = root.join("artifact");
    fs::write(
        &artifact,
        "#!/bin/sh\nprintf '%s' \"$RAKE_RELOAD_CHECKPOINT\"\nexec /bin/sleep 300\n",
    )
    .unwrap();
    fs::set_permissions(&artifact, fs::Permissions::from_mode(0o700)).unwrap();
    let args = Args {
        dev: true,
        json: true,
        ..Args::default()
    };
    let cancelled = AtomicBool::new(false);
    let session = root.join("session");
    fs::create_dir(&session).unwrap();
    let mut supervisor = Supervisor::new(&args, project(root), session, &cancelled).unwrap();
    supervisor.app = Some(
        Processes::new(&cancelled)
            .start(Command::new("/bin/sleep").arg("300"))
            .unwrap(),
    );
    let old_pid = supervisor.app.as_ref().unwrap().0.id();
    let mut instance = runtime.instance.clone();
    instance["pid"] = json!(old_pid);
    write_marker(&supervisor.control.join("runtime.json"), &instance).unwrap();
    fs::write(supervisor.control.join("runtime.sock"), "owned socket name").unwrap();
    assert!(!supervisor.publication(2, &artifact).unwrap());
    assert_eq!(supervisor.status["phase"], "reload_waiting");
    busy.store(false, Ordering::Release);
    assert!(supervisor.publication(2, &artifact).unwrap());
    assert_ne!(supervisor.app.as_ref().unwrap().0.id(), old_pid);
    assert_eq!(
        read_json(&supervisor.checkpoint).unwrap(),
        json!({"generation":7,"draft":"é漢字"})
    );
    assert_eq!(
        fs::metadata(&supervisor.checkpoint)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(fs::read_dir(&supervisor.control).unwrap().count(), 0);
    assert_eq!(supervisor.status["published"], 2);
    supervisor.finish().unwrap();
}

#[test]
fn cancellation_during_preparation_aborts_and_preserves_the_old_app() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let signal = cancelled.clone();
    let runtime = Runtime::new(move |value, index| {
        if value["type"] == "prepare_reload" {
            signal.store(true, Ordering::Release);
        }
        replies(value, index)
    });
    let root = runtime.directory.path();
    let args = Args {
        dev: true,
        json: true,
        ..Args::default()
    };
    let session = root.join("session");
    fs::create_dir(&session).unwrap();
    let mut supervisor = Supervisor::new(&args, project(root), session, &cancelled).unwrap();
    supervisor.app = Some(
        Processes::new(&cancelled)
            .start(Command::new("/bin/sleep").arg("300"))
            .unwrap(),
    );
    let mut instance = runtime.instance.clone();
    instance["pid"] = json!(supervisor.app.as_ref().unwrap().0.id());
    write_marker(&supervisor.control.join("runtime.json"), &instance).unwrap();
    let result = supervisor.publication(2, &root.join("artifact"));
    assert!(result.is_err());
    assert_ne!(supervisor.status["phase"], "reload_blocked");
    assert!(
        supervisor
            .app
            .as_mut()
            .unwrap()
            .0
            .try_wait()
            .unwrap()
            .is_none()
    );
    assert!(!supervisor.checkpoint.exists());
    let calls = runtime.calls.lock().unwrap();
    assert_eq!(calls.last().unwrap()["message"]["value"]["applied"], false);
    supervisor.finish().unwrap();
}

#[test]
fn restore_failure_is_reported_and_clears_after_runtime_recovery() {
    let failed = Arc::new(AtomicBool::new(true));
    let state = failed.clone();
    let runtime = Runtime::new(move |value, index| {
        if value["type"] == "reload" && state.load(Ordering::Acquire) {
            Some(json!({"phase":"restore_failed", "error":"incompatible checkpoint"}))
        } else {
            replies(value, index)
        }
    });
    let root = runtime.directory.path();
    let args = Args {
        dev: true,
        json: true,
        ..Args::default()
    };
    let cancelled = AtomicBool::new(false);
    let session = root.join("session");
    fs::create_dir(&session).unwrap();
    let mut supervisor = Supervisor::new(&args, project(root), session, &cancelled).unwrap();
    supervisor.app = Some(
        Processes::new(&cancelled)
            .start(Command::new("/bin/sleep").arg("300"))
            .unwrap(),
    );
    let mut instance = runtime.instance.clone();
    instance["pid"] = json!(supervisor.app.as_ref().unwrap().0.id());
    write_marker(&supervisor.control.join("runtime.json"), &instance).unwrap();
    supervisor
        .record("watching", json!({"published":1,"app_running":true}))
        .unwrap();
    supervisor.observe_app().unwrap();
    assert_eq!(supervisor.status["phase"], "restore_failed");
    assert_eq!(supervisor.status["error"], "incompatible checkpoint");
    failed.store(false, Ordering::Release);
    supervisor.observe_app().unwrap();
    assert_eq!(supervisor.status["phase"], "watching");
    assert!(supervisor.status["error"].is_null());
    assert_eq!(supervisor.status["reload"]["phase"], "ready");
    assert_eq!(supervisor.status["runtime_id"], "fixture-runtime");
    supervisor.finish().unwrap();
}

#[test]
fn dev_features_and_options_use_the_shared_native_supervisor() {
    let mut project = project(Path::new("/unused"));
    project.package = json!({"features":{}, "dependencies":[{"name":"rake-app","rename":"framework","kind":null}]});
    let mut args = Args {
        dev: true,
        features: Some("theme".into()),
        ..Args::default()
    };
    assert_eq!(
        project.build_features(&args).as_deref(),
        Some("theme,framework/devtools")
    );
    project.package["features"] = json!({"devtools":[]});
    assert_eq!(
        project.build_features(&args).as_deref(),
        Some("theme,devtools")
    );
    args.dev = false;
    assert_eq!(project.build_features(&args).as_deref(), Some("theme"));
    for (arguments, web, preview) in [
        (vec![], false, false),
        (vec!["--cdp"], false, false),
        (vec!["status", "--session", "/session"], false, false),
        (vec!["--target", "web"], true, false),
        (vec!["--t=web"], true, false),
        (vec!["--features=--preview"], false, false),
        (vec!["--target=web", "--target=desktop"], false, false),
    ] {
        let args = super::super::args::parse(
            "dev",
            &arguments
                .iter()
                .map(|argument| (*argument).to_owned())
                .collect::<Vec<_>>(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(args.web, web, "{arguments:?}");
        assert_eq!(args.preview, preview, "{arguments:?}");
        assert_eq!(args.port, if web { 8081 } else { 0 });
    }
    for flag in ["--preview", "--pre"] {
        let args = super::super::args::parse("dev", &[flag.to_owned()]);
        if cfg!(target_os = "macos") {
            assert!(args.unwrap().unwrap().preview);
        } else {
            assert!(args.is_err());
        }
    }
}

#[test]
fn build_output_keeps_empty_rendered_records_and_omits_artifact_json() {
    let cancelled = AtomicBool::new(false);
    let output = ProcessOutput::new();
    let sink = |text: &str, pid, stream: &str| {
        output
            .lock()
            .unwrap()
            .append(text, pid, stream, None, false)
    };
    Processes::new(&cancelled).with_output(&sink).run_output(Command::new("/bin/sh").args(["-c", r#"printf '%s\n' '{"reason":"compiler-message","message":{"rendered":""}}' '{"reason":"compiler-artifact"}' 'plain diagnostic'"#]), |_| {}).unwrap();
    let snapshot = output.lock().unwrap().snapshot();
    let records = snapshot["records"].as_array().unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["text"], "");
    assert_eq!(records[1]["text"], "plain diagnostic\n");
    assert!(
        records
            .iter()
            .all(|record| record["stream"] == "build" && record["pid"].as_u64().unwrap() > 0)
    );
}
