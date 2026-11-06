//! Covers: real panic persistence, next-launch delivery, concurrent processes and privacy.
//! Real: native panic hook, filesystem, collector and spawned test executable.
//! Doubles: file-writing application sink.
//! Requires: Linux/native host, no GPUI or display server.

#![cfg(not(target_family = "wasm"))]

use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};

use rake::crash::{CrashConfig, CrashLimits, CrashRecord};
use rake::diagnostics::Collector;

fn command(directory: &Path, operation: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().expect("test executable"));
    command
        .args(["--ignored", "--exact", "crash_child", "--nocapture"])
        .env("RAKE_CRASH_TEST_DIRECTORY", directory)
        .env("RAKE_CRASH_TEST_OPERATION", operation);
    command
}

fn crash(directory: &Path, operation: &str) -> std::process::Output {
    command(directory, operation)
        .output()
        .expect("spawn crash process")
}

fn records(directory: &Path) -> Vec<std::path::PathBuf> {
    fs::read_dir(directory)
        .expect("read directory")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json" || extension == "claimed")
        })
        .collect()
}

#[test]
#[ignore = "subprocess helper; parent tests supply an isolated directory and operation"]
fn crash_child() {
    let directory = std::path::PathBuf::from(
        std::env::var_os("RAKE_CRASH_TEST_DIRECTORY").expect("parent directory"),
    );
    let operation = std::env::var("RAKE_CRASH_TEST_OPERATION").expect("parent operation");
    let reporter = CrashConfig::new("test-1.2.3")
        .directory(&directory)
        .include_panic_message(operation.starts_with("message-"))
        .limits(CrashLimits {
            records: 128,
            ..CrashLimits::default()
        })
        .open("test.crash")
        .expect("open reporter");
    if operation == "deliver" {
        println!("READY");
        io::stdout().flush().expect("ready flush");
        io::stdin().read_exact(&mut [0]).expect("start signal");
        reporter
            .deliver(&move |record: &CrashRecord| {
                let output = directory.join("accepted");
                fs::create_dir_all(&output)?;
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(output.join(&record.id))?;
                file.write_all(&serde_json::to_vec(record)?)
            })
            .expect("deliver records");
        return;
    }
    let collector = Collector::new(16);
    collector.record(
        "ERROR",
        "private/path",
        "event",
        vec!["user title".into()],
        [("text".into(), serde_json::json!("SECRET_USER_CONTENT"))].into(),
    );
    reporter.install(collector).expect("install hook");
    assert_eq!(
        reporter
            .install(Collector::default())
            .expect_err("duplicate hook")
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    if operation == "threads" {
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(32));
        let threads: Vec<_> = (0..32)
            .map(|_| {
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    panic!("SECRET_USER_CONTENT");
                })
            })
            .collect();
        for thread in threads {
            assert!(thread.join().is_err());
        }
        return;
    }
    if operation == "broken" {
        fs::remove_dir_all(&directory).expect("remove directory");
        fs::write(&directory, "not a directory").expect("replace directory");
    }
    if operation == "caught" {
        assert!(std::panic::catch_unwind(|| panic!("SECRET_USER_CONTENT")).is_err());
        println!("CONTINUED_AFTER_PANIC");
        return;
    }
    if operation == "message-static" {
        panic!("static test panic");
    }
    if operation == "message-string" {
        std::panic::panic_any(String::from("owned test panic"));
    }
    if operation == "message-huge" {
        std::panic::panic_any("🦀".repeat(100_000));
    }
    if operation == "message-unknown" {
        std::panic::panic_any(42_u32);
    }
    if operation == "huge" {
        panic!("{}", "SECRET_USER_CONTENT🦀".repeat(100_000));
    }
    panic!("SECRET_USER_CONTENT");
}

fn consumer(directory: &Path) -> Child {
    let mut child = command(directory, "deliver")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn consumer");
    let mut output = io::BufReader::new(child.stdout.as_mut().expect("stdout"));
    let mut line = String::new();
    loop {
        line.clear();
        assert_ne!(
            output.read_line(&mut line).expect("ready line"),
            0,
            "consumer exited before ready"
        );
        if line.trim() == "READY" {
            break;
        }
    }
    child
}

fn finish(mut child: Child) {
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"x")
        .expect("release consumer");
    let output = child.wait_with_output().expect("wait for consumer");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn panic_is_delivered_on_second_run_and_absent_on_third() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let directory = temporary.path().join("crashes");
    let output = crash(&directory, "panic");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("SECRET_USER_CONTENT"),
        "previous hook still prints panic"
    );
    let paths = records(&directory);
    assert_eq!(paths.len(), 1);
    let bytes = fs::read(&paths[0]).expect("crash JSON");
    assert!(!String::from_utf8_lossy(&bytes).contains("SECRET_USER_CONTENT"));
    assert!(!String::from_utf8_lossy(&bytes).contains("user title"));
    let record: CrashRecord = serde_json::from_slice(&bytes).expect("valid record");
    assert_eq!(record.app_version, "test-1.2.3");
    assert_eq!(record.message, "Rust panic (payload omitted for privacy)");
    assert!(
        record
            .location
            .expect("panic location")
            .starts_with(concat!(file!(), ":"))
    );
    assert!(!record.backtrace.contains("/Github/"));
    assert_eq!(record.diagnostics.expect("tail").len(), 1);
    finish(consumer(&directory));
    assert!(records(&directory).is_empty());
    finish(consumer(&directory));
    assert_eq!(
        fs::read_dir(directory.join("accepted"))
            .expect("accepted directory")
            .count(),
        1
    );
}

#[test]
fn huge_panic_payload_is_never_serialized() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let directory = temporary.path().join("crashes");
    let output = crash(&directory, "huge");
    assert!(!output.status.success());
    let paths = records(&directory);
    assert_eq!(paths.len(), 1);
    let bytes = fs::read(&paths[0]).expect("record");
    assert!(bytes.len() <= 64 * 1024);
    assert!(!String::from_utf8_lossy(&bytes).contains("SECRET_USER_CONTENT"));
}

#[test]
fn concurrent_panics_and_two_delivering_processes_do_not_duplicate() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let directory = temporary.path().join("crashes");
    let output = crash(&directory, "threads");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(records(&directory).len(), 32);
    let mut first = consumer(&directory);
    let mut second = consumer(&directory);
    first
        .stdin
        .take()
        .expect("first stdin")
        .write_all(b"x")
        .expect("release first");
    second
        .stdin
        .take()
        .expect("second stdin")
        .write_all(b"x")
        .expect("release second");
    for child in [first, second] {
        let output = child.wait_with_output().expect("consumer exit");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(records(&directory).is_empty());
    assert_eq!(
        fs::read_dir(directory.join("accepted"))
            .expect("accepted")
            .count(),
        32
    );
}

#[test]
fn hook_io_failure_preserves_original_panic_without_recursive_abort() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let directory = temporary.path().join("crashes");
    let path = directory.join("crashes");
    let output = crash(&path, "broken");
    assert_eq!(
        output.status.code(),
        Some(101),
        "normal Rust panic exit, not abort signal"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("SECRET_USER_CONTENT"));
}

#[test]
fn opted_in_panic_messages_are_string_only_and_utf8_bounded() {
    for (operation, expected, truncated) in [
        ("message-static", "static test panic".to_owned(), false),
        ("message-string", "owned test panic".to_owned(), false),
        ("message-huge", "🦀".repeat(256), true),
        (
            "message-unknown",
            "Rust panic (payload omitted for privacy)".to_owned(),
            false,
        ),
    ] {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let directory = temporary.path().join("crashes");
        let output = crash(&directory, operation);
        assert_eq!(
            output.status.code(),
            Some(101),
            "{operation}: normal panic exit"
        );
        let paths = records(&directory);
        assert_eq!(paths.len(), 1, "{operation}: one record");
        let bytes = fs::read(&paths[0]).expect("record bytes");
        let record: CrashRecord = serde_json::from_slice(&bytes).expect("valid record");

        assert_eq!(record.message, expected, "{operation}: captured payload");
        assert!(record.message.len() <= 1024);
        if truncated {
            assert!(record.truncated);
        }
    }
}

#[test]
fn a_caught_panic_is_recorded_even_when_the_process_continues() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let directory = temporary.path().join("crashes");
    let output = crash(&directory, "caught");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("CONTINUED_AFTER_PANIC"));
    let paths = records(&directory);
    assert_eq!(paths.len(), 1);
    let record: CrashRecord = serde_json::from_slice(&fs::read(&paths[0]).expect("record bytes"))
        .expect("valid panic record");
    assert_eq!(record.message, "Rust panic (payload omitted for privacy)");
}
