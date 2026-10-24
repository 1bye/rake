//! Covers: terminal parity over real permission-restricted native local transport.
//! Real: CLI child process, private Unix socket, discovery, router and command protocol.
//! Doubles: headless route mounts. Requires: Unix local sockets and child-process execution.

#![cfg(unix)]

use futures::channel::oneshot;
use futures::future::select;
use rake::control::local::{self, LocalServer};
use rake::control::{Message, PROTOCOL_VERSION, Query, Reply, Request};
use rake::router::{Route, Router, RouterConfig};
use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::time::Duration;

#[derive(Clone, rake::Route)]
enum Destination {
    #[route(id = "home", path = "/")]
    Home,
}

#[rake::data(schema)]
#[derive(rake::Command)]
#[command(id = "test.echo", output = String, capability = Headless)]
#[serde(deny_unknown_fields)]
struct Echo {
    message: String,
}

fn cli(directory: &std::path::Path, arguments: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_rake"))
        .env("RAKE_CONTROL_DIR", directory)
        .args(arguments)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn terminal_submissions_survive_disconnect_retry_and_query_the_shared_runtime() {
    let directory = std::env::temp_dir().join(format!("rake-ctl-{}", uuid::Uuid::new_v4()));
    // Short socket paths are required on macOS.
    let directory = std::path::PathBuf::from("/tmp").join(directory.file_name().unwrap());
    let router = Router::<Destination>::new(RouterConfig::default());
    let client = router.client();

    client
        .commands()
        .register::<Echo, _, _>(|args, _| async move { Ok(args.message) })
        .unwrap();
    let gui_operation = match router
        .navigator()
        .dispatch(
            Echo {
                message: "shared".into(),
            },
            "gui-echo".into(),
        )
        .unwrap()
    {
        Reply::Submitted { operation_id } => operation_id,
        _ => panic!("submitted GUI command"),
    };

    let server = LocalServer::bind(&client, "test.parity", &directory).unwrap();

    assert_eq!(
        fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let instance = local::discover(&directory).unwrap().pop().unwrap();

    let local::Endpoint::Local { path } = instance.endpoint else {
        panic!()
    };

    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let owned_directory = directory.clone();
    let (sender, receiver) = oneshot::channel();

    let thread = std::thread::spawn(move || {
        let commands = cli(&owned_directory, &["ctl", "commands", "list", "--json"]);

        assert_eq!(commands["value"][0]["id"], "test.echo");
        assert_eq!(
            commands["value"][0]["arguments"]["properties"]["message"]["type"],
            "string"
        );
        assert_eq!(commands["value"][0]["result"]["type"], "string");
        let echoed = cli(
            &owned_directory,
            &[
                "ctl",
                "commands",
                "run",
                "test.echo",
                "--args",
                r#"{"message":"shared"}"#,
                "--request-key",
                "gui-echo",
                "--json",
            ],
        );

        assert_eq!(
            echoed["reply"]["value"]["operation_id"],
            serde_json::to_value(gui_operation).unwrap()
        );
        assert_eq!(echoed["reply"]["value"]["state"], "Committed");
        assert_eq!(echoed["reply"]["value"]["result"], "shared");

        let tree = cli(&owned_directory, &["ctl", "inspect", "tree", "--json"]);

        assert_eq!(tree["type"], "elements");
        assert_eq!(tree["value"]["status"], "unavailable");
        assert_eq!(tree["value"]["elements"], serde_json::json!([]));
        let unavailable = Command::new(env!("CARGO_BIN_EXE_rake"))
            .env("RAKE_CONTROL_DIR", &owned_directory)
            .args(["ctl", "inspect", "element", "element-1-0", "--json"])
            .output()
            .unwrap();

        assert!(!unavailable.status.success());
        assert!(String::from_utf8_lossy(&unavailable.stderr).contains("unavailable"));
        let invalid = Command::new(env!("CARGO_BIN_EXE_rake"))
            .env("RAKE_CONTROL_DIR", &owned_directory)
            .args(["ctl", "inspect", "unknown", "--json"])
            .output()
            .unwrap();

        assert!(!invalid.status.success());
        let error: Value = serde_json::from_slice(&invalid.stderr).unwrap();

        assert!(
            error["error"]["message"]
                .as_str()
                .unwrap()
                .contains("unsupported command")
        );

        let listing = cli(&owned_directory, &["instances", "--json"]);

        assert_eq!(listing.as_array().unwrap().len(), 1);
        let first = cli(
            &owned_directory,
            &[
                "ctl",
                "router",
                "open",
                "home",
                "--request-key",
                "retry",
                "--no-wait",
                "--json",
            ],
        );

        let again = cli(
            &owned_directory,
            &[
                "ctl",
                "router",
                "open",
                "home",
                "--request-key",
                "retry",
                "--json",
            ],
        );

        assert_eq!(
            first["reply"]["value"]["operation_id"],
            again["reply"]["value"]["operation_id"]
        );
        assert_eq!(again["reply"]["value"]["state"], "Committed");
        assert!(again["reply"]["value"]["result"]["entry_id"].is_number());
        assert_eq!(
            cli(&owned_directory, &["ctl", "tabs", "list", "--json"])["value"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let request = Request {
            protocol: PROTOCOL_VERSION,
            runtime_id: instance.info.runtime_id,
            key: Some("disconnect".into()),
            message: Message::Command(rake::control::Command::Open {
                destination: Destination::Home.encode(),
                options: rake::router::OpenOptions::new_tab(),
            }),
        };

        // Submit then disconnect before reading. The router must still execute it.
        use std::io::Write;
        let mut socket = std::os::unix::net::UnixStream::connect(&path).unwrap();
        let mut bytes = serde_json::to_vec(&request).unwrap();

        bytes.push(b'\n');
        socket.write_all(&bytes).unwrap();
        drop(socket);
        let reply = local::exchange(&path, &request, Duration::from_secs(5))
            .unwrap()
            .unwrap();

        assert!(matches!(reply, Reply::Submitted { .. }));
        let tabs = cli(&owned_directory, &["ctl", "tabs", "list", "--json"]);

        assert_eq!(tabs["value"].as_array().unwrap().len(), 2);
        let malformed = Request {
            protocol: 99,
            runtime_id: instance.info.runtime_id,
            key: None,
            message: Message::Query(Query::Info {}),
        };

        assert!(
            local::exchange(&path, &malformed, Duration::from_secs(5))
                .unwrap()
                .is_err()
        );
        sender.send(()).unwrap();
    });

    futures::executor::block_on(async {
        let work = async {
            let (routing, transport) = futures::join!(router.run(), server.run(client));

            routing.unwrap();
            transport.unwrap();
        };

        let check = async {
            receiver.await.unwrap();
        };

        select(Box::pin(work), Box::pin(check)).await;
    });
    router.shutdown();
    thread.join().unwrap();
    drop(server);
    assert!(fs::read_dir(&directory).unwrap().next().is_none());
    fs::remove_dir(directory).unwrap();
}

#[test]
fn insecure_or_symlink_discovery_directories_are_rejected() {
    let path =
        std::path::PathBuf::from("/tmp").join(format!("rake-perms-{}", uuid::Uuid::new_v4()));

    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        local::prepare_directory(&path).unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    let link = path.with_extension("link");

    std::os::unix::fs::symlink(&path, &link).unwrap();
    assert!(local::prepare_directory(&link).is_err());
    fs::remove_file(link).unwrap();
    fs::remove_dir(path).unwrap();
}
