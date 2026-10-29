//! Covers: standalone client discovery, socket exchanges, typed failures and broker validation.
//! Real: owned protocol, private filesystem discovery and Unix sockets.
//! Doubles: one-request protocol server. Requires: temporary Unix sockets and ephemeral loopback TCP ports.
#![cfg(unix)]

use rake_client::{Endpoint, Error, Instance, local, query};
use rake_protocol::control::{ControlError, MAX_MESSAGE_BYTES, Query, Reply, Request};
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "rake-client-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));

        local::prepare_directory(&path).unwrap();

        Self(path)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn instance(endpoint: Endpoint) -> Instance {
    Instance {
        application_id: "standalone-test".into(),
        pid: std::process::id(),
        endpoint,
        info: serde_json::from_value(serde_json::json!({
            "protocol": 1, "runtime_id": "00000000-0000-4000-8000-000000000001",
            "platform": "native", "commands": [], "queries": ["info"],
            "supported_targets": [], "completed_operations": 8, "pending_operations": 8,
            "idempotency_capacity": 8
        }))
        .unwrap(),
    }
}

#[test]
fn discovered_socket_supports_queries_without_the_application_runtime() {
    let directory = Directory::new();
    let path = directory.0.join("runtime.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let advertisement = instance(Endpoint::Local { path });

    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(directory.0.join("runtime.json"))
        .unwrap();

    serde_json::to_writer(file, &advertisement).unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();

        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut input = String::new();

        BufReader::new(&stream).read_line(&mut input).unwrap();
        let request: Request = serde_json::from_str(&input).unwrap();

        assert_eq!(request.runtime_id, advertisement.info.runtime_id);
        assert!(request.key.is_none());
        let reply = Ok::<_, ControlError>(Reply::Acknowledged);

        writeln!(stream, "{}", serde_json::to_string(&reply).unwrap()).unwrap();
    });

    let discovered = local::discover(&directory.0).unwrap();

    assert_eq!(discovered.len(), 1);
    assert!(matches!(
        query(&discovered[0], Query::Info {}, Duration::from_secs(2)).unwrap(),
        Reply::Acknowledged
    ));
    server.join().unwrap();
}

#[test]
fn socket_reply_limits_and_runtime_errors_remain_inspectable() {
    for (bytes, oversized) in [
        (
            format!(
                "{}\n",
                serde_json::to_string(&Err::<Reply, _>(ControlError::WrongRuntime)).unwrap()
            )
            .into_bytes(),
            false,
        ),
        (vec![b'x'; MAX_MESSAGE_BYTES + 1], true),
    ] {
        let directory = Directory::new();
        let path = directory.0.join("runtime.sock");
        let listener = UnixListener::bind(&path).unwrap();

        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();

            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut input = String::new();

            BufReader::new(&stream).read_line(&mut input).unwrap();
            stream.write_all(&bytes).unwrap();
        });

        let error = query(
            &instance(Endpoint::Local { path }),
            Query::Info {},
            Duration::from_secs(2),
        )
        .unwrap_err();

        if oversized {
            assert!(
                matches!(error, Error::Io(ref error) if error.kind() == std::io::ErrorKind::InvalidData)
            );
        } else {
            assert!(matches!(error, Error::Control(ControlError::WrongRuntime)));
        }

        server.join().unwrap();
    }
}

#[test]
fn invalid_brokers_are_rejected_before_credentials_can_be_sent() {
    for url in [
        "https://localhost/",
        "http://example.com/",
        "http://localhost/path",
        "http://localhost/?token=bad",
        "http://user:pass@localhost/",
        "http://localhost/#fragment",
        "not a URL",
    ] {
        let instance = instance(Endpoint::Web {
            url: url.into(),
            token: "secret".into(),
        });

        assert!(
            matches!(
                query(&instance, Query::Info {}, Duration::from_secs(1)),
                Err(Error::InvalidBroker)
            ),
            "{url}"
        );
    }
}

#[test]
fn web_exchange_normalizes_the_broker_path_and_preserves_authentication() {
    use std::io::Read;
    use std::net::TcpListener;

    for trailing_slash in ["", "/"] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}{trailing_slash}", listener.local_addr().unwrap());

        let advertisement = instance(Endpoint::Web {
            url,
            token: "test-token".into(),
        });

        let runtime = advertisement.info.runtime_id;

        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();

            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut reader = BufReader::new(&stream);
            let mut first = String::new();

            reader.read_line(&mut first).unwrap();
            assert_eq!(first, "POST /__rake/client HTTP/1.1\r\n");
            let mut length = None;
            let mut token = None;

            loop {
                let mut line = String::new();

                assert_ne!(reader.read_line(&mut line).unwrap(), 0);

                if line == "\r\n" {
                    break;
                }

                let (name, value) = line.split_once(':').unwrap();

                if name.eq_ignore_ascii_case("content-length") {
                    length = Some(value.trim().parse::<usize>().unwrap());
                }

                if name.eq_ignore_ascii_case("x-rake-token") {
                    token = Some(value.trim().to_string());
                }
            }

            let mut bytes = vec![0; length.unwrap()];

            reader.read_exact(&mut bytes).unwrap();
            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

            assert_eq!(token.as_deref(), Some("test-token"));
            assert_eq!(body["runtime_id"], serde_json::to_value(runtime).unwrap());
            assert_eq!(body["request"]["message"]["value"]["type"], "info");
            let reply = serde_json::to_string(&Ok::<_, ControlError>(Reply::Acknowledged)).unwrap();

            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            )
            .unwrap();
        });

        assert!(matches!(
            query(&advertisement, Query::Info {}, Duration::from_secs(2)).unwrap(),
            Reply::Acknowledged
        ));
        server.join().unwrap();
    }
}
