//! Covers: encoded byte budgets, retention order, interrupted claims and file policy.
//! Real: filesystem, JSON and native advisory locks.
//! Doubles: synthetic backtraces and application sinks.
//! Requires: private temporary directory on a native filesystem.

use super::*;

fn reporter(limits: CrashLimits) -> (tempfile::TempDir, CrashReporter) {
    let directory = tempfile::tempdir().expect("temporary directory");

    let reporter = CrashConfig::new("1.2.3")
        .directory(directory.path().join("crashes"))
        .limits(limits)
        .open("test.crash")
        .expect("open crash directory");

    (directory, reporter)
}

#[test]
fn huge_backtrace_and_json_escaping_fit_encoded_budget() {
    let (_directory, reporter) = reporter(CrashLimits {
        record_bytes: 1024,
        ..CrashLimits::default()
    });

    let mut record = reporter.record(None, None);

    record.backtrace = "\"\\\n🦀".repeat(100_000);
    record.app_version = "\n".repeat(128);
    record.location = Some("\n".repeat(256));
    reporter.write(record).expect("write bounded record");
    let path = reporter.records().expect("records").remove(0);
    let bytes = fs::read(path).expect("record bytes");

    assert!(bytes.len() <= 1024);
    let record: CrashRecord = serde_json::from_slice(&bytes).expect("valid bounded JSON");

    assert!(record.truncated);
    assert_eq!(record.message, PRIVATE_MESSAGE);
}

#[test]
fn trace_formatting_is_bounded_and_utf8_safe() {
    let mut trace = TraceText {
        text: String::new(),
        truncated: false,
    };

    for _ in 0..100_000 {
        let _ = writeln!(trace, "🦀\"");
    }

    assert!(trace.text.len() <= MAX_BACKTRACE_BYTES);
    assert!(trace.truncated);
}

#[test]
fn count_limit_drops_oldest() {
    let (_directory, reporter) = reporter(CrashLimits {
        records: 2,
        ..CrashLimits::default()
    });

    for id in ["001", "002", "003", "004"] {
        let mut record = reporter.record(None, None);

        record.id = id.into();
        reporter.write(record).expect("write record");
    }

    let paths = reporter.records().expect("records");

    assert_eq!(paths.len(), 2);
    assert_eq!(paths[0].file_stem().expect("stem"), "003");
    assert_eq!(paths[1].file_stem().expect("stem"), "004");
}

#[test]
fn failing_and_unwinding_sinks_retain_record_and_interrupted_claims_retry() {
    let (_directory, reporter) = reporter(CrashLimits::default());

    reporter
        .write(reporter.record(None, None))
        .expect("write record");
    assert!(
        reporter
            .deliver(&|_: &CrashRecord| Err(io::Error::other("retry")))
            .is_err()
    );
    assert_eq!(reporter.records().expect("records").len(), 1);
    let unwind = std::panic::catch_unwind(|| {
        let _ = reporter.deliver(&|_: &CrashRecord| -> io::Result<()> { panic!("sink failed") });
    });

    assert!(unwind.is_err());
    let pending = reporter.records().expect("records").remove(0);

    fs::rename(&pending, pending.with_extension("claimed")).expect("simulate interrupted claim");
    assert_eq!(
        reporter.deliver(&|_: &CrashRecord| Ok(())).expect("retry"),
        1
    );
    assert_eq!(
        reporter.deliver(&|_: &CrashRecord| Ok(())).expect("empty"),
        0
    );
}

#[test]
fn corrupt_truncated_oversized_and_invalid_old_versions_are_skipped() {
    let (_directory, reporter) = reporter(CrashLimits {
        record_bytes: 1024,
        ..CrashLimits::default()
    });

    for (name, bytes) in [
        ("bad", b"not JSON".to_vec()),
        ("partial", b"{\"schema_version\":".to_vec()),
        ("large", vec![b' '; 2048]),
    ] {
        let mut file = private_options()
            .write(true)
            .create_new(true)
            .open(reporter.directory.join(format!("{name}.json")))
            .expect("private file");

        file.write_all(&bytes).expect("write invalid data");
    }

    let mut record = reporter.record(None, None);

    record.schema_version = 0;
    reporter.write(record).expect("invalid old schema");
    assert_eq!(
        reporter
            .deliver(&|_: &CrashRecord| panic!("corrupt record reached sink"))
            .expect("skip"),
        0
    );
    assert!(reporter.records().expect("records").is_empty());
}

#[test]
fn invalid_config_is_rejected() {
    for id in ["", "..", "../escape", "/tmp", "a b"] {
        assert!(CrashConfig::new("1").open(id).is_err());
    }

    for limits in [
        CrashLimits {
            records: 0,
            ..CrashLimits::default()
        },
        CrashLimits {
            record_bytes: 1023,
            ..CrashLimits::default()
        },
    ] {
        let directory = tempfile::tempdir().expect("temporary directory");

        assert!(
            CrashConfig::new("1")
                .directory(directory.path())
                .limits(limits)
                .open("test")
                .is_err()
        );
    }
}

#[cfg(unix)]
#[test]
fn permissions_and_symlinks_follow_storage_rules() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let (directory, reporter) = reporter(CrashLimits::default());

    reporter
        .write(reporter.record(None, None))
        .expect("write record");
    let path = reporter.records().expect("records").remove(0);

    assert_eq!(
        fs::metadata(&path).expect("metadata").permissions().mode() & 0o777,
        0o600
    );
    let alias = directory.path().join("alias");

    symlink(&reporter.directory, &alias).expect("directory symlink");
    assert!(CrashConfig::new("1").directory(alias).open("test").is_err());
    fs::remove_file(&path).expect("remove record");
    let outside = directory.path().join("outside");

    fs::write(&outside, b"private sentinel").expect("outside file");
    symlink(&outside, &path).expect("record symlink");
    assert!(reporter.deliver(&|_: &CrashRecord| Ok(())).is_err());
    assert_eq!(fs::read(&outside).expect("sentinel"), b"private sentinel");
    fs::set_permissions(&reporter.directory, fs::Permissions::from_mode(0o755))
        .expect("public directory");
    assert!(
        CrashConfig::new("1")
            .directory(&reporter.directory)
            .open("test")
            .is_err()
    );
}

#[test]
fn concurrent_writers_leave_bounded_valid_files() {
    let (_directory, reporter) = reporter(CrashLimits {
        records: 4,
        ..CrashLimits::default()
    });

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));

    let workers: Vec<_> = (0..16)
        .map(|_| {
            let reporter = reporter.clone();
            let barrier = barrier.clone();

            std::thread::spawn(move || {
                let record = reporter.record(None, None);

                barrier.wait();
                reporter.write(record).expect("concurrent write");
            })
        })
        .collect();

    for worker in workers {
        worker.join().expect("writer exit");
    }

    reporter
        .prune()
        .expect("trim after concurrent writes settle");
    let paths = reporter.records().expect("records");

    assert_eq!(paths.len(), 4);
    assert!(!paths.is_empty());
    assert!(paths.len() <= 4, "{} retained records", paths.len());

    for path in paths {
        let record: CrashRecord =
            serde_json::from_slice(&fs::read(path).expect("record bytes")).expect("valid record");

        assert_eq!(record.schema_version, 1);
    }
}

#[test]
fn version_and_location_are_utf8_bounded_and_marked_truncated() {
    let directory = tempfile::tempdir().expect("temporary directory");

    let reporter = CrashConfig::new("🦀".repeat(100))
        .directory(directory.path().join("crashes"))
        .open("test.crash")
        .expect("open reporter");

    let record = reporter.record(Some("🦀".repeat(100)), None);

    assert_eq!(record.app_version.len(), 128);
    assert_eq!(record.location.expect("location").len(), 256);
    assert!(record.truncated);
}

#[test]
fn malformed_record_id_is_not_passed_to_sink() {
    let (_directory, reporter) = reporter(CrashLimits::default());
    let mut record = reporter.record(None, None);

    reporter.write(record.clone()).expect("write record");
    let path = reporter.records().expect("records").remove(0);

    record.id = "../outside".into();
    let bytes = serde_json::to_vec(&record).expect("record JSON");

    private_options()
        .write(true)
        .truncate(true)
        .open(path)
        .expect("open file")
        .write_all(&bytes)
        .expect("malformed ID");

    assert_eq!(
        reporter
            .deliver(&|_: &CrashRecord| panic!("unsafe ID reached sink"))
            .expect("skip malformed ID"),
        0
    );
}

#[test]
fn opted_in_message_is_trimmed_before_backtrace_for_encoded_budget() {
    let (_directory, reporter) = reporter(CrashLimits {
        record_bytes: 1024,
        ..CrashLimits::default()
    });

    let mut record = reporter.record(None, None);

    record.message = "🦀".repeat(256);
    record.backtrace = "B".repeat(250);
    reporter.write(record).expect("write bounded payload");
    let bytes = fs::read(reporter.records().expect("records").remove(0)).expect("record bytes");
    let record: CrashRecord = serde_json::from_slice(&bytes).expect("valid bounded JSON");

    assert!(bytes.len() <= 1024);
    assert!(record.message.len() < 1024);
    assert_eq!(record.backtrace, "B".repeat(250));
    assert!(record.truncated);
}

#[test]
fn newer_schemas_survive_delivery_and_interrupted_claims_with_unknown_layouts() {
    for extension in ["json", "claimed"] {
        let (_directory, reporter) = reporter(CrashLimits::default());
        let future = reporter.directory.join(format!("000-future.{extension}"));
        let bytes = br#"{"schema_version":2,"future_layout":{"value":"preserve this"}}"#;

        private_options()
            .write(true)
            .create_new(true)
            .open(&future)
            .expect("future file")
            .write_all(bytes)
            .expect("future JSON");
        reporter
            .write(reporter.record(None, None))
            .expect("current-schema record");

        assert_eq!(
            reporter
                .deliver(&|record: &CrashRecord| {
                    assert_eq!(record.schema_version, 1);
                    Ok(())
                })
                .expect("deliver supported records"),
            1
        );
        let pending = future.with_extension("json");

        assert_eq!(fs::read(&pending).expect("preserved bytes"), bytes);
        assert_eq!(
            reporter
                .deliver(&|_: &CrashRecord| panic!("future schema reached sink"))
                .expect("skip future schema again"),
            0
        );
        assert_eq!(reporter.records().expect("records"), vec![pending]);
    }
}

#[test]
fn newer_schema_records_are_still_subject_to_oldest_first_retention() {
    let (_directory, reporter) = reporter(CrashLimits {
        records: 1,
        ..CrashLimits::default()
    });

    let mut future = reporter.record(None, None);

    future.id = "001".into();
    future.schema_version = 99;
    reporter.write(future).expect("write future record");
    assert_eq!(
        reporter
            .deliver(&|_: &CrashRecord| panic!("future schema reached sink"))
            .expect("preserve future"),
        0
    );
    assert_eq!(reporter.records().expect("future remains").len(), 1);
    let mut current = reporter.record(None, None);

    current.id = "002".into();
    reporter
        .write(current)
        .expect("newer capture prunes oldest");

    assert_eq!(
        reporter
            .deliver(&|record: &CrashRecord| {
                assert_eq!(record.id, "002");
                Ok(())
            })
            .expect("deliver newest"),
        1
    );
    assert!(reporter.records().expect("records").is_empty());
}

#[test]
fn relative_source_paths_are_kept_and_absolute_build_roots_are_removed() {
    for (input, expected) in [
        (
            "crates/rake/src/storage/mod.rs",
            "crates/rake/src/storage/mod.rs",
        ),
        (
            r"crates\rake\src\storage\mod.rs",
            r"crates\rake\src\storage\mod.rs",
        ),
        (
            "/home/private/.cargo/registry/serde-1.0/src/de/mod.rs",
            "serde-1.0/src/de/mod.rs",
        ),
        (
            r"C:\Users\private\registry\serde-1.0\src\de\mod.rs",
            "serde-1.0/src/de/mod.rs",
        ),
        (r"\\server\private\dep\src\lib.rs", "dep/src/lib.rs"),
        (
            "/home/private/src/workspace/pkg/src/nested/mod.rs",
            "pkg/src/nested/mod.rs",
        ),
        ("/home/private/build/generated.rs", "generated.rs"),
        (r"C:\Users\private\generated.rs", "generated.rs"),
        ("/src/lib.rs", "lib.rs"),
        ("lib.rs", "lib.rs"),
    ] {
        assert_eq!(source_path(input), expected, "source path {input}");
    }
}

#[test]
fn scope_exit_unlocks_even_while_an_inherited_description_is_alive() {
    let (_directory, reporter) = reporter(CrashLimits::default());
    let path = reporter.directory.join("delivery.lock");

    let file = private_options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .expect("lock file");

    file.try_lock().expect("initial lock");
    let guard = FileLock(&file);
    // dup shares the open file description just like a descriptor inherited by fork.
    let inherited = file.try_clone().expect("inherited description");

    drop(guard);
    drop(file);
    let next = private_options()
        .read(true)
        .write(true)
        .open(path)
        .expect("reopen");

    next.try_lock().expect("scope exit released inherited lock");
    drop(inherited);
}

#[test]
fn retention_notification_survives_open_handles_and_concurrent_pruning() {
    let (_directory, reporter) = reporter(CrashLimits {
        records: 4,
        ..CrashLimits::default()
    });

    let path = reporter.directory.join("retention.pending");

    // Keep a writer's notification handle alive across the owner's clear/scan.
    let pending = private_options()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .expect("notification");

    for round in 0..16 {
        std::thread::scope(|scope| {
            for writer in 0..8 {
                let reporter = &reporter;

                scope.spawn(move || {
                    let mut record = reporter.record(None, None);

                    record.id = format!("{round:04}-{writer:04}");
                    reporter.write(record).expect("concurrent notification");
                });
            }
        });
        assert_eq!(reporter.records().expect("records").len(), 4);
        assert_eq!(fs::metadata(&path).expect("stable notification").len(), 0);
        assert_eq!(pending.metadata().expect("held notification").len(), 0);
    }
}
