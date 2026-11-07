//! Covers: App's pre-GPUI recording policy, opt-out, setup failures and later delivery.
//! Real: startup component called by App::run, panic hook, filesystem, child processes.
//! Doubles: file receipt sink; GPUI is neither linked nor initialized.
//! Requires: native host; each child has isolated app-data environment variables.

#![cfg(not(target_family = "wasm"))]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tracing_subscriber::prelude::*;

use rake::crash::{CrashConfig, CrashSink};
use rake::diagnostics::Collector;
use rake::{crash, crash_config, diagnostics};
use std::io;

// Exercise the exact private startup component without exporting test-only API.
#[path = "../src/crash/startup.rs"]
mod startup;
use crate::crash::CrashRecord;
use startup::{CrashReports, version_from_policy};

const APP_ID: &str = "test.default-crash";

fn directory(root: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    let base = root.join("Library/Application Support");
    #[cfg(not(target_os = "macos"))]
    let base = root.join("data");

    base.join(APP_ID).join("crashes")
}

fn run(root: &Path, scenario: &str) -> Output {
    Command::new(std::env::current_exe().expect("test executable"))
        .args(["--ignored", "--exact", "startup_child", "--nocapture"])
        .env("RAKE_CRASH_STARTUP_ROOT", root)
        .env("RAKE_CRASH_STARTUP_SCENARIO", scenario)
        .env("XDG_DATA_HOME", root.join("data"))
        .env("APPDATA", root.join("data"))
        .env("HOME", root)
        .output()
        .expect("startup child")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn records(path: &Path) -> Vec<CrashRecord> {
    if !path.exists() {
        return Vec::new();
    }

    fs::read_dir(path)
        .expect("crash directory")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .map(|path| {
            serde_json::from_slice(&fs::read(path).expect("record bytes")).expect("valid record")
        })
        .collect()
}

#[test]
#[ignore = "subprocess helper; parent tests isolate hook installation and app-data roots"]
fn startup_child() {
    let root = PathBuf::from(std::env::var_os("RAKE_CRASH_STARTUP_ROOT").expect("parent root"));
    let scenario = std::env::var("RAKE_CRASH_STARTUP_SCENARIO").expect("parent scenario");
    let collector = Collector::new(16);
    let subscriber = tracing_subscriber::registry().with(collector.layer());
    let mut policy = CrashReports::default();

    if scenario.contains("open-failure") {
        let path = directory(&root);
        let parent = path.parent().expect("app directory");
        fs::create_dir_all(parent.parent().expect("data root")).expect("data root");
        fs::write(parent, b"a file prevents creating the crash directory")
            .expect("block directory");
    }

    if scenario.contains("hook-conflict") {
        CrashConfig::new("host")
            .directory(root.join("host"))
            .open(APP_ID)
            .expect("host directory")
            .install(collector.clone())
            .expect("host hook");
    }

    if scenario.starts_with("explicit")
        || scenario == "configure-last"
        || scenario == "disable-last"
    {
        if scenario == "configure-last" {
            policy.disable();
        }
        let config = if scenario == "configure-last" || scenario == "disable-last" {
            crate::crash_config!().directory(root.join("explicit"))
        } else {
            crate::crash_config!()
        };
        let receipts = root.join("accepted");
        policy.configure(config, move |record: &CrashRecord| {
            fs::create_dir_all(&receipts)?;
            use std::io::Write;
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(receipts.join(&record.id))?
                .write_all(b"accepted")
        });
    }

    if scenario == "disabled" || scenario == "disable-last" {
        policy.disable();
    }
    let version = if scenario == "default-version-panic" {
        version_from_policy(Some(
            r#"{"version":"consumer-2.3.4","package_version":"2.3.4+package"}"#,
        ))
    } else {
        None
    };
    let result = tracing::subscriber::with_default(subscriber, || {
        policy.initialize(APP_ID, version.as_deref(), collector.clone())
    });

    if scenario.starts_with("explicit") && scenario.ends_with("failure") {
        assert!(result.is_err(), "explicit setup must fail startup");
        return;
    }
    if scenario == "explicit-hook-conflict" {
        assert_eq!(
            result.expect_err("explicit install failure").kind(),
            io::ErrorKind::AlreadyExists
        );
        return;
    }
    result.expect("startup continues");

    if scenario.starts_with("default")
        && (scenario.contains("failure") || scenario.contains("conflict"))
    {
        assert!(
            collector
                .snapshot()
                .records
                .iter()
                .any(|record| record.level == "WARN"),
            "default setup failure must emit a warning"
        );
    }
    if scenario == "disabled" || scenario == "disable-last" {
        assert!(!directory(&root).exists());
        assert!(!root.join("explicit").exists());
        CrashConfig::new("host")
            .directory(root.join("host"))
            .open(APP_ID)
            .expect("host directory")
            .install(collector)
            .expect("opt-out installed no hook");
    }
    if scenario.ends_with("panic")
        || scenario == "configure-last"
        || scenario == "default-hook-conflict"
    {
        panic!("SECRET_USER_CONTENT");
    }
}

#[test]
fn default_startup_records_without_a_sink_and_uses_available_consumer_version() {
    for (scenario, version) in [
        ("default-panic", "unknown"),
        ("default-version-panic", "consumer-2.3.4"),
    ] {
        let root = tempfile::tempdir().expect("temporary root");
        let output = run(root.path(), scenario);

        assert_eq!(output.status.code(), Some(101), "normal panic exit");
        let reports = records(&directory(root.path()));
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].app_version, version);
        assert_eq!(
            reports[0].message,
            "Rust panic (payload omitted for privacy)"
        );
        assert!(!root.path().join("accepted").exists());
    }
}

#[test]
fn opt_out_creates_no_directory_and_leaves_hook_installation_to_the_host() {
    let root = tempfile::tempdir().expect("temporary root");

    assert_success(&run(root.path(), "disabled"));
    assert!(!directory(root.path()).exists());
}

#[test]
fn default_setup_errors_warn_and_continue_but_explicit_setup_errors_fail() {
    for scenario in [
        "default-open-failure",
        "explicit-open-failure",
        "explicit-hook-conflict",
    ] {
        let root = tempfile::tempdir().expect("temporary root");
        assert_success(&run(root.path(), scenario));
    }
    let root = tempfile::tempdir().expect("temporary root");
    let output = run(root.path(), "default-hook-conflict");

    assert_eq!(
        output.status.code(),
        Some(101),
        "startup succeeded and reached deliberate panic"
    );
    assert!(records(&directory(root.path())).is_empty());
    assert_eq!(
        records(&root.path().join("host")).len(),
        1,
        "host hook remains installed"
    );
}

#[test]
fn last_builder_policy_call_wins() {
    let disabled = tempfile::tempdir().expect("temporary root");
    assert_success(&run(disabled.path(), "disable-last"));
    assert!(!directory(disabled.path()).exists());
    assert!(!disabled.path().join("explicit").exists());
    let configured = tempfile::tempdir().expect("temporary root");
    let output = run(configured.path(), "configure-last");

    assert_eq!(output.status.code(), Some(101));
    assert!(!directory(configured.path()).exists());
    let reports = records(&configured.path().join("explicit"));
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].app_version, env!("CARGO_PKG_VERSION"));
}

#[test]
fn default_records_wait_until_a_later_launch_adds_a_sink_and_deliver_once() {
    let root = tempfile::tempdir().expect("temporary root");
    assert_eq!(run(root.path(), "default-panic").status.code(), Some(101));
    let pending = records(&directory(root.path())).remove(0).id;

    assert_success(&run(root.path(), "default-only"));
    assert_eq!(
        records(&directory(root.path()))[0].id,
        pending,
        "no sink leaves records waiting"
    );
    assert_success(&run(root.path(), "explicit-deliver"));
    assert!(records(&directory(root.path())).is_empty());
    assert_success(&run(root.path(), "explicit-deliver"));
    assert_eq!(
        fs::read_dir(root.path().join("accepted"))
            .expect("receipts")
            .count(),
        1
    );
    assert!(root.path().join("accepted").join(pending).exists());
}

#[test]
fn version_capture_uses_existing_policy_and_missing_or_invalid_metadata_falls_back() {
    assert_eq!(
        version_from_policy(Some(r#"{"version":"2.3.4"}"#)).as_deref(),
        Some("2.3.4")
    );
    for policy in [
        None,
        Some("{}"),
        Some("not JSON"),
        Some(r#"{"version":""}"#),
        Some(r#"{"version":"   "}"#),
    ] {
        assert_eq!(version_from_policy(policy), None);
    }
}
