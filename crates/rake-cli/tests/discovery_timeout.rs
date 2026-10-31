//! Covers: the configured CLI timeout also governs runtime identity discovery.
//! Real: CLI process, private discovery files and Unix socket frames.
//! Doubles: a deliberately delayed identity peer. Requires: Unix sockets.

#![cfg(unix)]

use rake::control::local::{self, Endpoint, Instance};
use rake::router::{NoRoute, Router, RouterConfig};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::process::Command;
use std::time::{Duration, Instant};

#[test]
fn explicit_timeout_allows_a_slow_identity_probe() {
    let directory =
        std::path::PathBuf::from("/tmp").join(format!("rake-probe-{}", uuid::Uuid::new_v4()));

    local::prepare_directory(&directory).unwrap();
    let path = directory.join("control.sock");
    let listener = UnixListener::bind(&path).unwrap();

    listener.set_nonblocking(true).unwrap();
    let router = Router::<NoRoute>::new(RouterConfig::default());

    let instance = Instance {
        application_id: "test.slow-probe".into(),
        pid: std::process::id(),
        info: router.client().info(),
        endpoint: Endpoint::Local { path },
    };

    let manifest = directory.join("instance.json");

    std::fs::write(&manifest, serde_json::to_vec(&instance).unwrap()).unwrap();
    std::fs::set_permissions(&manifest, std::fs::Permissions::from_mode(0o600)).unwrap();
    let reply =
        serde_json::to_vec(&serde_json::json!({"Ok": {"type": "info", "value": instance.info}}))
            .unwrap();

    let peer = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);

        for index in 0..2 {
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(_) => return,
                }
            };

            // Accepted sockets can inherit the listener's nonblocking mode on macOS.
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(4)))
                .unwrap();
            let mut frame = String::new();

            BufReader::new(&stream).read_line(&mut frame).unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&frame).unwrap()["message"]["value"]["type"],
                "info"
            );

            if index == 0 {
                // Reproduce Chrome's throttled background-tab response, beyond the old 2 s probe.
                std::thread::sleep(Duration::from_millis(2300));
            }

            if stream
                .write_all(&reply)
                .and_then(|_| stream.write_all(b"\n"))
                .is_err()
            {
                return;
            }
        }
    });

    let output = Command::new(env!("CARGO_BIN_EXE_rake"))
        .env("RAKE_CONTROL_DIR", &directory)
        .args([
            "ctl",
            "--instance",
            "test.slow-probe",
            "--timeout",
            "4",
            "--json",
            "info",
        ])
        .output()
        .unwrap();

    peer.join().unwrap();
    std::fs::remove_dir_all(&directory).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["type"],
        "info"
    );
}
