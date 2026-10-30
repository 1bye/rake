//! Real root/descendant ownership, without shell commands or arbitrary delays.
#![cfg(any(unix, windows))]
use fs2::FileExt;
use rake_resources::HelperProcess;
use std::{
    fs,
    io::{BufRead, Write},
    process::{Command, Stdio},
};
#[test]
#[expect(
    clippy::zombie_processes,
    reason = "fixture deliberately abandons a descendant to test tree cleanup"
)]
fn helper_fixture() {
    let Some(root) = std::env::var_os("RAKE_HELPER_PROCESS_TEST") else {
        return;
    };
    if std::env::var_os("RAKE_HELPER_LEAF").is_none() {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "helper_fixture", "--nocapture"])
            .env("RAKE_HELPER_LEAF", "1")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut reader = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut ready = false;
        for _ in 0..20 {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap() == 0 {
                break;
            }
            if line.contains("HELPER_LEAF_READY") {
                ready = true;
                break;
            }
        }
        assert!(ready, "descendant failed before acquiring lock");
        // Deliberately abandon the child to verify process-tree cleanup by the owner.
        println!("HELPER_TREE_READY");
        std::io::stdout().flush().unwrap();
        loop {
            std::thread::park();
        }
    }
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(std::path::PathBuf::from(root).join("owned.lock"))
        .unwrap();
    lock.lock_exclusive().unwrap();
    println!("HELPER_LEAF_READY");
    std::io::stdout().flush().unwrap();
    loop {
        std::thread::park();
    }
}
#[test]
fn dropping_a_helper_releases_its_descendants_lock() {
    let root = tempfile::tempdir().unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "helper_fixture", "--nocapture"])
        .env("RAKE_HELPER_PROCESS_TEST", root.path())
        .stdout(Stdio::piped());
    let mut helper = HelperProcess::spawn(command).unwrap();
    let mut reader = std::io::BufReader::new(helper.take_stdout().unwrap());
    let mut ready = false;
    for _ in 0..20 {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap() == 0 {
            break;
        }
        if line.contains("HELPER_TREE_READY") {
            ready = true;
            break;
        }
    }
    assert!(ready, "helper failed before descendant was running");
    let lock = fs::OpenOptions::new()
        .write(true)
        .open(root.path().join("owned.lock"))
        .unwrap();
    assert!(lock.try_lock_exclusive().is_err());
    drop(helper);
    // Reaping the root does not synchronize an orphaned descendant's OS teardown.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match lock.try_lock_exclusive() {
            Ok(()) => break,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "dropping owner left descendant alive: {error}"
                );
                std::thread::yield_now();
            }
            Err(error) => panic!("could not inspect descendant lock: {error}"),
        }
    }
}
