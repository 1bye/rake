//! Covers: cancellation owns descendant processes, including Drop cleanup.
//! Real: native process groups/Windows Jobs, file locks, child test processes.
//! Doubles: none. Requires: permission to spawn native child processes.
use fs2::FileExt;
use rake_dist::Worker;
use std::{
    fs,
    process::Command,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

#[test]
fn child_fixture() {
    if let Some(path) = std::env::var_os("RAKE_DIST_CHILD_LOCK") {
        let file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .unwrap();
        file.lock_exclusive().unwrap();
        fs::write(
            std::path::PathBuf::from(path).with_extension("ready"),
            "ready",
        )
        .unwrap();
        std::thread::park_timeout(Duration::from_secs(60));
    }
}

#[test]
fn worker_fixture() {
    if std::env::var_os("RAKE_DIST_WORKER_FIXTURE").is_some() {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "child_fixture"])
            .spawn()
            .unwrap();
        child.wait().unwrap();
    }
}

#[test]
fn cancellation_releases_a_descendants_lock() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("child.lock");
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "worker_fixture"])
        .env("RAKE_DIST_CHILD_LOCK", &path)
        .env("RAKE_DIST_WORKER_FIXTURE", "1");
    let mut worker = Worker::spawn(&mut command).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.with_extension("ready").exists() {
        assert!(Instant::now() < deadline, "descendant did not start");
        std::thread::sleep(Duration::from_millis(10));
    }
    let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
    assert!(
        file.try_lock_exclusive().is_err(),
        "child must own lock before cancellation"
    );
    let error = worker
        .wait(&AtomicBool::new(true), Duration::from_secs(10))
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
    while file.try_lock_exclusive().is_err() {
        assert!(
            Instant::now() < deadline,
            "descendant survived cancellation"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
