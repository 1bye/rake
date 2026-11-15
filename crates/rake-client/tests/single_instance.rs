//! Covers: crash recovery, concurrent ownership, real activation transport.
//! Real: kernel locks, private files, local sockets and named pipes.
//! Doubles: none. Requires: desktop OS.
#![cfg(any(unix, windows))]

use std::{
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[test]
fn child() {
    let Ok(mode) = std::env::var("RAKE_INSTANCE_TEST") else {
        return;
    };
    let primary =
        rake_client::single::launch("single-test", &["app://open/document".into()]).unwrap();
    if let Some(primary) = primary {
        println!("primary");
        use std::io::Write;
        std::io::stdout().flush().unwrap();
        if mode == "crash" {
            std::process::exit(0);
        }

        let arguments = futures::executor::block_on(primary.receive()).unwrap();
        assert_eq!(arguments, ["app://open/document"]);
    } else {
        println!("secondary");
    }
}

fn spawn(root: &std::path::Path, mode: &str) -> std::process::Child {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child", "--nocapture"])
        .env("RAKE_INSTANCE_TEST", mode)
        .env("RAKE_CONTROL_DIR", root)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap()
}

fn finish(mut child: std::process::Child) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            panic!("instance child timed out");
        }

        std::thread::sleep(Duration::from_millis(10));
    }

    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn stale_lock_and_simultaneous_launches() {
    #[cfg(unix)]
    let root = std::path::PathBuf::from(format!("/tmp/rake-si-test-{}", std::process::id()));
    #[cfg(windows)]
    let root = std::env::temp_dir().join(format!("rake-si-test-{}", std::process::id()));
    let crashed = finish(spawn(&root, "crash"));
    assert!(crashed.contains("primary"));

    let first = spawn(&root, "serve");
    let second = spawn(&root, "serve");
    let first = finish(first);
    let second = finish(second);
    assert_eq!(
        usize::from(first.contains("primary")) + usize::from(second.contains("primary")),
        1
    );
    assert_eq!(
        usize::from(first.contains("secondary")) + usize::from(second.contains("secondary")),
        1
    );
    std::fs::remove_dir_all(root).unwrap();
}
