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

#[test]
fn nested_deadlines_cannot_extend_or_leak_into_later_operations() {
    use std::time::Instant;
    let instance = instance(Endpoint::Local {
        path: PathBuf::from("/unused-expired-deadline.sock"),
    });

    let expired = Instant::now() - Duration::from_secs(1);

    assert!(rake_client::current_deadline().is_none());
    rake_client::with_deadline(expired, || {
        assert_eq!(rake_client::current_deadline(), Some(expired));
        rake_client::with_deadline(Instant::now() + Duration::from_secs(10), || {
            assert_eq!(rake_client::current_deadline(), Some(expired));
            assert!(
                matches!(query(&instance, Query::Info {}, Duration::from_secs(2)), Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::TimedOut)
            );
        });
    });
    assert!(rake_client::current_deadline().is_none());
    assert!(
        matches!(query(&instance, Query::Info {}, Duration::from_secs(2)), Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound)
    );
}

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

#[test]
fn debug_output_redacts_endpoint_and_instance_credentials() {
    let advertisement = instance(Endpoint::Web {
        url: "http://127.0.0.1:9999".into(),
        token: "private-session-token".into(),
    });

    for debug in [
        format!("{:?}", advertisement.endpoint),
        format!("{advertisement:#?}"),
    ] {
        assert!(
            !debug.contains("private-session-token"),
            "credential appeared in Debug output"
        );
        assert!(debug.contains("REDACTED"));
        assert!(debug.contains("127.0.0.1"));
    }
}

fn read_http_request(stream: &std::net::TcpStream) {
    use std::io::Read;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut reader = BufReader::new(stream);
    let mut length = 0;

    loop {
        let mut line = String::new();

        assert_ne!(reader.read_line(&mut line).unwrap(), 0);

        if line == "\r\n" {
            break;
        }

        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap();
        }
    }

    let mut body = vec![0; length];

    reader.read_exact(&mut body).unwrap();
}

#[test]
fn broker_redirects_never_forward_credentials_to_another_endpoint() {
    use std::net::TcpListener;
    let target = TcpListener::bind("127.0.0.1:0").unwrap();

    target.set_nonblocking(true).unwrap();
    let target_url = format!("http://{}/unexpected", target.local_addr().unwrap());
    let broker = TcpListener::bind("127.0.0.1:0").unwrap();

    let advertisement = instance(Endpoint::Web {
        url: format!("http://{}", broker.local_addr().unwrap()),
        token: "secret".into(),
    });

    let server = std::thread::spawn(move || {
        let (mut stream, _) = broker.accept().unwrap();

        read_http_request(&stream);
        write!(stream, "HTTP/1.1 307 Temporary Redirect\r\nLocation: {target_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
    });

    let result = query(&advertisement, Query::Info {}, Duration::from_millis(300));

    server.join().unwrap();
    assert!(matches!(
        result,
        Err(Error::Http(rake_http::Error::Status { status: 307 }))
    ));
    assert!(
        matches!(target.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
        "redirect opened a second connection"
    );
}

#[test]
fn web_control_replies_obey_the_same_byte_limit_as_local_frames() {
    use std::net::TcpListener;

    for chunked in [false, true] {
        let broker = TcpListener::bind("127.0.0.1:0").unwrap();

        let advertisement = instance(Endpoint::Web {
            url: format!("http://{}", broker.local_addr().unwrap()),
            token: "secret".into(),
        });

        let body = serde_json::to_vec(&Ok::<_, ControlError>(Reply::Data(serde_json::json!(
            "x".repeat(MAX_MESSAGE_BYTES)
        ))))
        .unwrap();

        let server = std::thread::spawn(move || {
            let (mut stream, _) = broker.accept().unwrap();

            read_http_request(&stream);
            stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let result = if chunked {
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
                )
                .and_then(|()| {
                    for chunk in body.chunks(8192) {
                        write!(stream, "{:x}\r\n", chunk.len())?;
                        stream.write_all(chunk)?;
                        stream.write_all(b"\r\n")?;
                    }

                    stream.write_all(b"0\r\n\r\n")
                })
            } else {
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .and_then(|()| stream.write_all(&body))
            };

            // A bounded reader may close before the server finishes writing.
            if let Err(error) = result {
                assert!(matches!(
                    error.kind(),
                    std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
                ));
            }
        });

        let result = query(&advertisement, Query::Info {}, Duration::from_secs(2));

        server.join().unwrap();
        assert!(
            matches!(result, Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::InvalidData),
            "chunked={chunked}"
        );
    }
}

#[test]
fn socket_deadline_covers_the_entire_reply_even_when_bytes_keep_arriving() {
    let directory = Directory::new();
    let path = directory.0.join("slow.sock");
    let listener = UnixListener::bind(&path).unwrap();

    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();

        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = String::new();

        BufReader::new(&stream).read_line(&mut request).unwrap();
        // Each byte arrives within the read timeout, but the full frame exceeds it.
        let reply = serde_json::to_vec(&Ok::<_, ControlError>(Reply::Acknowledged)).unwrap();

        for byte in reply.into_iter().chain([b'\n']) {
            if stream.write_all(&[byte]).is_err() {
                break;
            }

            std::thread::sleep(Duration::from_millis(40));
        }
    });

    let result = query(
        &instance(Endpoint::Local { path }),
        Query::Info {},
        Duration::from_millis(250),
    );

    server.join().unwrap();
    assert!(
        matches!(result, Err(Error::Io(error)) if matches!(error.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock))
    );
}

#[test]
fn web_reply_at_the_byte_limit_still_decodes() {
    use std::net::TcpListener;
    let broker = TcpListener::bind("127.0.0.1:0").unwrap();

    let advertisement = instance(Endpoint::Web {
        url: format!("http://{}", broker.local_addr().unwrap()),
        token: "secret".into(),
    });

    let overhead = serde_json::to_vec(&Ok::<_, ControlError>(Reply::Data(serde_json::json!(""))))
        .unwrap()
        .len();

    let length = MAX_MESSAGE_BYTES - overhead;

    let body = serde_json::to_vec(&Ok::<_, ControlError>(Reply::Data(serde_json::json!(
        "x".repeat(length)
    ))))
    .unwrap();

    assert_eq!(body.len(), MAX_MESSAGE_BYTES);
    let server = std::thread::spawn(move || {
        let (mut stream, _) = broker.accept().unwrap();

        read_http_request(&stream);
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(&body).unwrap();
    });

    let result = query(&advertisement, Query::Info {}, Duration::from_secs(2));

    server.join().unwrap();

    let Reply::Data(value) = result.unwrap() else {
        panic!("data reply")
    };

    assert_eq!(value.as_str().unwrap().len(), length);
}

#[test]
fn socket_deadline_also_cancels_a_blocked_write() {
    let directory = Directory::new();
    let path = directory.0.join("blocked.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let (release, released) = std::sync::mpsc::channel();

    let server = std::thread::spawn(move || {
        let (_stream, _) = listener.accept().unwrap();

        // Keep the socket open without draining it until the client deadline wins.
        released.recv_timeout(Duration::from_secs(3)).unwrap();
    });

    let advertisement = instance(Endpoint::Local { path: path.clone() });

    let request = Request {
        protocol: 1,
        runtime_id: advertisement.info.runtime_id,
        key: Some("blocked-write".into()),
        message: rake_protocol::control::Message::Command(rake_protocol::control::Command::Run {
            id: "fixture".into(),
            arguments: serde_json::json!("x".repeat(MAX_MESSAGE_BYTES / 2)),
        }),
    };

    let result = local::exchange(&path, &request, Duration::from_millis(250));

    release.send(()).unwrap();
    server.join().unwrap();
    assert!(matches!(result, Err(error) if error.kind() == std::io::ErrorKind::TimedOut));
}

#[test]
fn invalid_timeouts_and_oversized_requests_fail_before_connecting() {
    let directory = Directory::new();
    let path = directory.0.join("absent.sock");
    let advertisement = instance(Endpoint::Local { path: path.clone() });

    let mut request = Request {
        protocol: 1,
        runtime_id: advertisement.info.runtime_id,
        key: None,
        message: rake_protocol::control::Message::Query(Query::Info {}),
    };

    for timeout in [Duration::ZERO, Duration::MAX] {
        let result = local::exchange(&path, &request, timeout);

        assert!(matches!(result, Err(error) if error.kind() == std::io::ErrorKind::InvalidInput));
    }

    request.key = Some("x".repeat(MAX_MESSAGE_BYTES));
    let result = local::exchange(&path, &request, Duration::from_secs(1));

    assert!(matches!(result, Err(error) if error.kind() == std::io::ErrorKind::InvalidInput));
}
