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
        target_directory: root.join("target"),
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
                            let reply = if let Some(error) = response.get("$error") {
                                json!({"Err":error})
                            } else {
                                json!({"Ok":{"value":response}})
                            };
                            let _ = writeln!(connection, "{}", reply);
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
        "snapshot" => json!({"tabs":[{"entries":[{"id":1}]}]}),
        "reload" => json!({"phase":"ready"}),
        "operations" => json!([]),
        "checkpoint" | "partial_checkpoint" => json!({"generation":7,"draft":"é漢字"}),
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

#[test]
fn checkpoint_files_preserve_utf8_byte_bounds() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("checkpoint.json");
    let value = json!({"draft":"é漢字".repeat(20000)});

    write_marker(&path, &value).unwrap();

    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(&path).unwrap()).unwrap(),
        value
    );
    assert!(fs::metadata(&path).unwrap().len() < 200000);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn blaze_arguments_are_optional_and_reject_web_preview_and_other_commands() {
    use super::super::args::{self, Compiler};
    let parse = |args: &[&str]| {
        args::parse(
            "dev",
            &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        )
    };
    assert_eq!(parse(&[]).unwrap().unwrap().compiler, Compiler::Cargo);
    assert_eq!(
        parse(&["--compiler", "blaze"]).unwrap().unwrap().compiler,
        Compiler::Blaze
    );
    for arguments in [
        vec!["--compiler=blaze", "--target=web"],
        vec!["--preview", "--compiler=blaze"],
    ] {
        let arguments: Vec<_> = arguments.iter().map(|s| s.to_string()).collect();
        assert_eq!(
            args::parse("dev", &arguments).err().unwrap(),
            "--compiler blaze requires desktop without --preview"
        );
    }
    assert!(args::parse("build", &["--compiler=blaze".into()]).is_err());
    assert_eq!(
        parse(&["--compiler=blaze", "--native-reloader=/tmp/backend"])
            .err()
            .unwrap(),
        "--compiler blaze cannot be combined with --native-reloader"
    );
    assert!(!args::web_target(&["--preview".into()]));
    assert!(args::web_target(&["--target=web".into()]));
}

fn macho(path: &Path, uuid: u8) {
    let mut bytes = vec![0u8; 56];
    bytes[..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
    bytes[16..20].copy_from_slice(&1u32.to_le_bytes());
    bytes[20..24].copy_from_slice(&24u32.to_le_bytes());
    bytes[32..36].copy_from_slice(&0x1bu32.to_le_bytes());
    bytes[36..40].copy_from_slice(&24u32.to_le_bytes());
    bytes[40..56].fill(uuid);
    fs::write(path, bytes).unwrap();
}

#[test]
fn blaze_watched_artifact_uses_uuid_and_rejects_ambiguity_and_truncated_tables() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path();
    fs::create_dir_all(target.join("debug")).unwrap();
    fs::create_dir_all(target.join("blaze/whole/a")).unwrap();
    macho(&target.join("debug/app"), 1);
    macho(&target.join("blaze/whole/a/app"), 1);
    assert_eq!(
        blaze::artifact(target, "debug", "app").unwrap(),
        target.join("blaze/whole/a/app")
    );
    fs::create_dir_all(target.join("blaze/whole/b")).unwrap();
    macho(&target.join("blaze/whole/b/app"), 1);
    assert!(blaze::artifact(target, "debug", "app").is_err());
    fs::write(target.join("debug/app"), [0xcf, 0xfa, 0xed, 0xfe]).unwrap();
    assert!(blaze::identity(&target.join("debug/app")).is_err());
}

#[test]
fn blaze_daemon_wire_escapes_source_and_rejects_failed_truncated_and_oversized_replies() {
    for reply in [
        b"ok\n".to_vec(),
        b"error\tbad\\nsource\n".to_vec(),
        b"ok".to_vec(),
        vec![b'x'; MAX_BYTES + 1],
    ] {
        let dir = tempfile::Builder::new()
            .prefix("blaze-wire-")
            .tempdir_in("/tmp")
            .unwrap();
        let socket = dir.path().join("daemon.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let success = reply == b"ok\n";
        let thread = thread::spawn(move || {
            let (mut connection, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(&connection).read_line(&mut request).unwrap();
            assert_eq!(
                request,
                "edit\t/workspace/main.rs\tfn main() {\\n\\tprintln!(\"a\\\\b\");\\n}\n"
            );
            let _ = connection.write_all(&reply);
        });
        let cancelled = AtomicBool::new(false);
        let processes = Processes::new(&cancelled);
        let result = blaze::edit(
            &socket,
            Path::new("/workspace/main.rs"),
            "fn main() {\n\tprintln!(\"a\\b\");\n}",
            &processes,
        );
        assert_eq!(result.is_ok(), success);
        thread.join().unwrap();
    }
}

#[test]
fn blaze_runtime_outcomes_and_uuid_choose_patch_rebuild_or_checkpointed_restart_without_replay() {
    use blaze::{BuildKind, Decision, Live};
    let mut live = Live {
        identity: Some("uuid:first".into()),
        ..Live::default()
    };
    assert_eq!(
        live.decision("uuid:first", BuildKind::Edit),
        Decision::Patch
    );
    assert_eq!(
        live.decision("uuid:other", BuildKind::Edit),
        Decision::Restart
    );
    let restart =
        json!({"sequence":1,"kind":"live_restart_requested","fields":{"reason":"room used up"}});
    assert!(live.observe(&restart));
    assert!(!live.observe(&restart));
    assert_eq!(
        live.decision("uuid:first", BuildKind::Edit),
        Decision::Rebuild
    );
    assert_eq!(
        live.decision("uuid:first", BuildKind::Full),
        Decision::Restart
    );
    assert!(live.observe(
        &json!({"sequence":2,"kind":"live_patch_declined","fields":{"reason":"declined"}})
    ));
    assert!(
        live.observe(&json!({"sequence":3,"kind":"live_patch_applied","fields":{"sequence":100}}))
    );
    assert!(live.restart.is_none());
    assert!(live.declined.is_none());
    assert_eq!(
        live.decision("uuid:first", BuildKind::Edit),
        Decision::Patch
    );
}

#[test]
fn blaze_stat_watcher_detects_atomic_saves_and_deletions() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let source = dir.path().join("main.rs");
    fs::write(&source, "one").unwrap();
    let first = fingerprint::stamps(&project, None).unwrap();
    fs::write(dir.path().join("replacement"), "two").unwrap();
    fs::rename(dir.path().join("replacement"), &source).unwrap();
    let second = fingerprint::stamps(&project, None).unwrap();
    assert_ne!(first, second);
    fs::remove_file(source).unwrap();
    assert_ne!(second, fingerprint::stamps(&project, None).unwrap());
}

#[test]
fn blaze_environment_keeps_explicit_memory_limits_and_feature_aliases() {
    use std::ffi::OsString;
    let env =
        blaze::environment([(OsString::from("BLAZE_MEMORY_MAX"), OsString::from("8G"))].into());
    assert_eq!(
        env.get(std::ffi::OsStr::new("BLAZE_MEMORY_MAX")).unwrap(),
        "8G"
    );
    assert_eq!(
        env.get(std::ffi::OsStr::new("BLAZE_PROC_MACRO_SERVER"))
            .unwrap(),
        "0"
    );
    let dir = tempfile::tempdir().unwrap();
    let mut project = project(dir.path());
    project.package["dependencies"] = json!([{"name":"rake-app","rename":"framework","kind":null}]);
    let args = Args {
        dev: true,
        compiler: super::super::Compiler::Blaze,
        ..Args::default()
    };
    assert_eq!(
        project.build_features(&args).as_deref(),
        Some("framework/devtools,framework/live")
    );
}

#[test]
fn blaze_full_boundary_prepares_then_aborts_on_failure_even_during_cancellation() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("builds-root")).unwrap();
    let args = Args {
        compiler: super::super::Compiler::Blaze,
        ..Args::default()
    };
    let cancelled = AtomicBool::new(false);
    let runtime = Runtime::new(replies);
    let mut supervisor = Supervisor::new(
        &args,
        project(root.path()),
        root.path().join("builds-root"),
        &cancelled,
    )
    .unwrap();
    supervisor.app = Some(
        Processes::new(&cancelled)
            .start(Command::new("/bin/sleep").arg("300"))
            .unwrap(),
    );
    let mut advertisement = runtime.instance.clone();
    advertisement["pid"] = json!(supervisor.app.as_ref().unwrap().0.id());
    write_marker(&supervisor.control.join("app.json"), &advertisement).unwrap();
    supervisor.live.as_mut().unwrap().full_boundary = true;
    supervisor.prepare_live_boundary().unwrap();
    cancelled.store(true, Ordering::Release);
    supervisor.finish_live_boundary(false).unwrap();
    let calls = runtime.calls.lock().unwrap();
    let prepare = calls
        .iter()
        .find(|v| v["message"]["value"]["type"] == "prepare_reload")
        .unwrap();
    let finish = calls
        .iter()
        .find(|v| v["message"]["value"]["type"] == "finish_reload")
        .unwrap();
    assert_eq!(
        prepare["message"]["value"]["request"]["mechanism"],
        "native_code_replacement"
    );
    assert_eq!(
        prepare["message"]["value"]["request"]["id"],
        finish["message"]["value"]["id"]
    );
    assert_eq!(finish["message"]["value"]["applied"], false);
}

#[test]
fn blaze_patch_area_restart_requires_full_build_before_same_uuid_checkpointed_restart() {
    let root = tempfile::tempdir().unwrap();
    let session = root.path().join("session");
    fs::create_dir(&session).unwrap();
    let artifact = root.path().join("app");
    fs::write(&artifact, "#!/bin/sh\nexec /bin/sleep 300\n").unwrap();
    fs::set_permissions(&artifact, fs::Permissions::from_mode(0o755)).unwrap();
    let args = Args {
        compiler: super::super::Compiler::Blaze,
        ..Args::default()
    };
    let cancelled = AtomicBool::new(false);
    let runtime = Runtime::new(replies);
    let mut supervisor = Supervisor::new(&args, project(root.path()), session, &cancelled).unwrap();
    supervisor.launch(&artifact).unwrap();
    let pid = supervisor.app.as_ref().unwrap().0.id();
    let mut advertisement = runtime.instance.clone();
    advertisement["pid"] = json!(pid);
    write_marker(&supervisor.control.join("app.json"), &advertisement).unwrap();
    let live = supervisor.live.as_mut().unwrap();
    live.restart = Some("the room the program set aside for edits is used up".into());
    live.kind = blaze::BuildKind::Edit;
    supervisor.publish(2, &artifact).unwrap();
    assert!(supervisor.live.as_ref().unwrap().needs_build);
    assert_eq!(supervisor.app.as_ref().unwrap().0.id(), pid);
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
    supervisor.live.as_mut().unwrap().kind = blaze::BuildKind::Full;
    supervisor.publish(3, &artifact).unwrap();
    assert_ne!(supervisor.app.as_ref().unwrap().0.id(), pid);
    assert_eq!(read_json(&supervisor.checkpoint).unwrap()["draft"], "é漢字");
    supervisor.finish().unwrap();
}

#[test]
fn blaze_metadata_signature_changes_for_manifests_not_body_edits() {
    let root = tempfile::tempdir().unwrap();
    let project = project(root.path());
    fs::write(root.path().join("Cargo.toml"), "manifest").unwrap();
    let initial = blaze::configuration(&project).unwrap();
    fs::write(root.path().join("helper.rs"), "one").unwrap();
    assert_eq!(blaze::configuration(&project).unwrap(), initial);
    fs::write(root.path().join("Cargo.toml"), "different").unwrap();
    assert_ne!(blaze::configuration(&project).unwrap(), initial);
}

#[test]
fn pinned_live_mode_is_opt_in_and_configures_its_own_compiler() {
    use super::super::args::{self, Compiler};
    let plain = args::parse("dev", &[]).unwrap().unwrap();
    assert_eq!(plain.compiler, Compiler::Cargo);
    assert!(!plain.live);
    assert!(plain.blaze_binary.is_none());
    let mut live = args::parse("dev", &["--live".into()]).unwrap().unwrap();
    assert_eq!(live.compiler, Compiler::Blaze);
    assert!(live.live);
    for args in [
        vec!["--live", "--target=web"],
        vec!["--live", "--preview"],
        vec!["--live", "--compiler=cargo"],
    ] {
        assert!(
            args::parse(
                "dev",
                &args.iter().map(|s| s.to_string()).collect::<Vec<_>>()
            )
            .is_err()
        );
    }
    assert!(args::parse("build", &["--live".into()]).is_err());
    live.blaze_binary = Some(std::path::PathBuf::from("/private/pinned/blaze"));
    let mut command = std::process::Command::new(live.blaze_binary.as_ref().unwrap());
    blaze::configure(&mut command, &live);
    assert_eq!(command.get_program(), "/private/pinned/blaze");
    let env: std::collections::BTreeMap<_, _> = command
        .get_envs()
        .map(|(k, v)| {
            (
                k.to_string_lossy().to_string(),
                v.unwrap().to_string_lossy().to_string(),
            )
        })
        .collect();
    for (key, value) in [
        ("BLAZE_PROC_MACRO_WASM", "0"),
        ("BLAZE_PROC_MACRO_SERVER", "0"),
        ("LIBLZMA_NO_PKG_CONFIG", "1"),
        ("RUST_MIN_STACK", "67108864"),
        (
            "BLAZE_TOOLCHAIN_PROC_MACRO_SERVER",
            "/private/pinned/rust-analyzer-proc-macro-srv",
        ),
    ] {
        assert_eq!(env[key], value, "{key}");
    }
}

#[test]
fn permanent_live_restart_failure_waits_for_another_edit_but_busy_retries() {
    for busy in [false, true] {
        let runtime = Runtime::new(move |value, _| match value["type"].as_str().unwrap() {
            "diagnostics" => Some(json!({"records":[]})),
            "snapshot" => Some(json!({"tabs":[{"entries":[{"id":1}]}]})),
            "reload" => Some(json!({"phase":"idle"})),
            "operations" if busy => Some(json!([["op-1", "Running"]])),
            "operations" => Some(json!([])),
            // A disconnected prepare request exercises the permanent-error path.
            "prepare_reload" => None,
            _ => Some(Value::Null),
        });
        let root = runtime.directory.path();
        let artifact = root.join("artifact");
        fs::write(&artifact, "fixture executable").unwrap();
        let args = Args {
            dev: true,
            json: true,
            compiler: super::super::Compiler::Blaze,
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
        let live = supervisor.live.as_mut().unwrap();
        live.identity = Some(blaze::identity(&artifact).unwrap());
        live.restart = Some("view layout changed".into());
        live.sequence = 7;
        live.published = 6;

        assert_eq!(supervisor.publication(2, &artifact).unwrap(), !busy);
        assert_eq!(
            supervisor.live.as_ref().unwrap().published,
            if busy { 6 } else { 7 }
        );
        assert_eq!(supervisor.live.as_ref().unwrap().restart.is_some(), busy);
        assert_eq!(supervisor.app.as_ref().unwrap().0.id(), pid);
        assert_eq!(
            supervisor.status["phase"],
            if busy {
                "reload_waiting"
            } else {
                "reload_blocked"
            }
        );

        if !busy {
            // A later edit's distinct outcome must still be allowed to request restart.
            let live = supervisor.live.as_mut().unwrap();
            assert!(live.observe(&json!({"sequence":1,"kind":"live_restart_requested","fields":{"reason":"new edit"}})));
            assert!(live.restart.is_some());
            assert!(supervisor.publication(3, &artifact).unwrap());
            assert!(supervisor.live.as_ref().unwrap().restart.is_none());
        }
    }
}

#[test]
fn strict_blaze_blocks_uncheckpointed_root_restart_but_default_restarts() {
    use super::super::Compiler;
    for (compiler, require_state, blocked) in [
        (Compiler::Blaze, false, false),
        (Compiler::Blaze, true, true),
        (Compiler::Cargo, false, false),
    ] {
        let runtime = Runtime::new(|value, index| {
            if value["type"] == "snapshot" {
                Some(json!({"tabs":[]}))
            } else {
                replies(value, index)
            }
        });
        let root = runtime.directory.path();
        let args = Args {
            dev: true,
            json: true,
            compiler,
            require_state,
            ..Args::default()
        };
        let cancelled = AtomicBool::new(false);
        let session = root.join("session");
        fs::create_dir(&session).unwrap();
        let mut supervisor = Supervisor::new(&args, project(root), session, &cancelled).unwrap();
        let result = supervisor.prepare(&runtime.instance, &root.join("artifact"));
        if blocked {
            let error = result.unwrap_err().to_string();
            assert!(
                error.contains("standalone root state is not checkpointed"),
                "{error}"
            );
            assert!(error.contains("--require-state"), "{error}");
            assert!(
                !runtime
                    .calls
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|call| call["message"]["value"]["type"] == "prepare_reload")
            );
        } else {
            assert!(!result.unwrap());
            assert!(supervisor.checkpoint.exists());
        }
    }
}

#[test]
fn default_restart_hands_off_a_notice_and_preserves_available_checkpoints() {
    for case in ["root", "partial", "full"] {
        let checkpoint = if case == "root" {
            json!({"generation":1,"tabs":[]})
        } else {
            json!({"generation":1,"tabs":[{"entries":[
                {"id":2,"destination":{"route":"home"},"state":if case=="partial" {Value::Null} else {json!({"schema":"home","data":{"count":7}})}},
                {"id":4,"destination":{"route":"other"},"state":{"schema":"other","data":{"count":42}}}
            ]}]})
        };
        let captured = checkpoint.clone();
        let runtime = Runtime::new(move |value, index| {
            if value["type"] == "partial_checkpoint" {
                Some(captured.clone())
            } else {
                replies(value, index)
            }
        });
        let root = runtime.directory.path();
        let artifact = root.join("artifact");
        fs::write(&artifact, "#!/bin/sh\nprintf '%s' \"$RAKE_LIVE_RESTART_NOTICE\" > notice.txt\nexec /bin/sleep 300\n").unwrap();
        fs::set_permissions(&artifact, fs::Permissions::from_mode(0o755)).unwrap();
        let args = Args {
            dev: true,
            json: true,
            compiler: Compiler::Blaze,
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
        supervisor.live.as_mut().unwrap().restart = Some(
            "the edit changes the definition of `struct Toolkit`, which has old values".into(),
        );
        supervisor.publish(2, &artifact).unwrap();
        let pid = supervisor.app.as_ref().unwrap().0.id();
        assert_ne!(pid, old_pid);
        assert_eq!(read_json(&supervisor.checkpoint).unwrap(), checkpoint);
        let message: String = if case == "full" {
            "".into()
        } else if case == "root" {
            "restarted fixture: field change in Toolkit; app state was not preserved (no checkpoint registered)".into()
        } else {
            "restarted fixture: field change in Toolkit; app state was not preserved for Home (entry-2); registered checkpoints restored".into()
        };
        if case == "full" {
            assert!(supervisor.status["restart_notice"].is_null());
        } else {
            assert_eq!(supervisor.status["restart_notice"], message);
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while !root.join("notice.txt").exists() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            fs::read_to_string(root.join("notice.txt")).unwrap(),
            message
        );
        supervisor.finish().unwrap();
    }
}

#[test]
fn strict_state_check_precedes_daemon_relink_and_aborts_its_probe() {
    for case in ["root", "missing", "full"] {
        let runtime = Runtime::new(move |value, index| {
            if value["type"] == "snapshot" && case == "root" {
                Some(json!({"tabs":[]}))
            } else if value["type"] == "checkpoint" && case == "missing" {
                Some(json!({"$error":{"Reload":{"MissingState":2}}}))
            } else {
                replies(value, index)
            }
        });
        let root = runtime.directory.path();
        let artifact = root.join("artifact");
        fs::write(&artifact, "old executable").unwrap();
        let args = Args {
            dev: true,
            json: true,
            compiler: Compiler::Blaze,
            require_state: true,
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
        let live = supervisor.live.as_mut().unwrap();
        live.identity = Some(blaze::identity(&artifact).unwrap());
        live.kind = blaze::BuildKind::Edit;
        live.restart = Some("field change".into());
        assert!(supervisor.publication(2, &artifact).unwrap());
        assert_eq!(supervisor.app.as_ref().unwrap().0.id(), pid);
        assert_eq!(
            supervisor.live.as_ref().unwrap().needs_build,
            case == "full"
        );
        assert!(
            !supervisor.checkpoint.exists(),
            "a probe must not replace the recovery checkpoint"
        );
        if case == "full" {
            assert_eq!(supervisor.status["phase"], "rebuild_required");
        } else {
            assert_eq!(supervisor.status["phase"], "reload_blocked");
            assert!(supervisor.live.as_ref().unwrap().restart.is_none());
        }
        if case == "missing" {
            assert_eq!(
                supervisor.status["error"],
                "{\"Reload\":{\"MissingState\":2}}"
            );
        }
        let calls = runtime.calls.lock().unwrap();
        let aborted = calls.iter().any(|call| {
            call["message"]["value"]["type"] == "finish_reload"
                && call["message"]["value"]["applied"] == false
        });
        assert_eq!(aborted, case != "root");
        drop(calls);
        supervisor.finish().unwrap();
    }
}
