//! Covers: private launch validation and the bounded shared HTTP query protocol.
//! Real: Rake HTTP over TCP and JSON codecs. Doubles: a local inspector broker.
//! Requires: loopback sockets; no display or GPU.

use rake::control::{MAX_MESSAGE_BYTES, Query};
use rake::http::CancellationToken;
use rake_inspector::{Connection, ConnectionConfig};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

#[test]
fn launch_credentials_cannot_go_to_nonlocal_or_ambiguous_urls() {
    let token = "x".repeat(40);

    for prefix in [
        "https://example.com/",
        "http://example.com/",
        "http://user:password@localhost/",
        "http://localhost/?token=oops",
        "file:///tmp/",
        "http://localhost/arbitrary",
    ] {
        assert!(
            ConnectionConfig::from_launch_url(&format!("{prefix}#__rake_control={token}")).is_err()
        );
    }

    assert!(
        ConnectionConfig::from_launch_url(
            "http://127.0.0.1:8000/__rake/inspector#__rake_control=short"
        )
        .is_err()
    );
    assert_eq!(
        ConnectionConfig::from_launch_url(&format!(
            "http://127.0.0.1:8000/__rake/inspector#__rake_control={token}"
        ))
        .unwrap()
        .origin(),
        "http://127.0.0.1:8000"
    );
}

#[test]
fn sends_authentication_as_a_header_and_bounds_response_bytes() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();

    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();

        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut byte = [0];

        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
            assert!(request.len() < 8192);
        }

        let headers = String::from_utf8(request).unwrap().to_ascii_lowercase();

        assert!(headers.starts_with("post /__rake/inspect_instances http/1.1"));
        assert!(headers.contains(&format!("x-rake-token: {}", "x".repeat(40))));
        assert!(!headers.contains("__rake_control="));
        let length: usize = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length: "))
            .unwrap()
            .parse()
            .unwrap();

        let mut request_body = vec![0; length];

        stream.read_exact(&mut request_body).unwrap();
        let response = vec![b' '; MAX_MESSAGE_BYTES + 1];

        write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n", response.len()).unwrap();
        let _ = stream.write_all(&response);
    });

    let config = ConnectionConfig::from_launch_url(&format!(
        "http://{address}/__rake/inspector#__rake_control={}",
        "x".repeat(40)
    ))
    .unwrap();

    let connection = Connection::new(config).unwrap();
    let result = futures::executor::block_on(connection.instances(CancellationToken::new()));

    assert!(result.unwrap_err().contains("frame limit"));
    server.join().unwrap();
}

#[test]
fn non_elements_queries_are_rejected_without_network_access() {
    let config = ConnectionConfig::from_launch_url(&format!(
        "http://127.0.0.1:1/__rake/inspector#__rake_control={}",
        "x".repeat(40)
    ))
    .unwrap();

    let connection = Connection::new(config).unwrap();

    let runtime =
        serde_json::from_value(serde_json::json!("00000000-0000-4000-8000-000000000001")).unwrap();

    let result = futures::executor::block_on(connection.query(
        runtime,
        Query::Snapshot {},
        CancellationToken::new(),
    ));

    assert!(result.unwrap_err().contains("only supports Elements"));
    let result = futures::executor::block_on(connection.control(
        runtime,
        rake::control::Command::Run {
            id: "not.allowed".into(),
            arguments: serde_json::json!({}),
        },
        CancellationToken::new(),
    ));

    assert!(
        result
            .unwrap_err()
            .contains("only supports development inspection controls")
    );
}

#[test]
fn exact_runtime_query_roundtrips_the_shared_protocol() {
    let runtime =
        serde_json::from_value(serde_json::json!("00000000-0000-4000-8000-000000000001")).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();

    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();

        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut headers = Vec::new();
        let mut byte = [0];

        while !headers.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            headers.push(byte[0]);
            assert!(headers.len() < 8192);
        }

        let headers = String::from_utf8(headers).unwrap().to_ascii_lowercase();

        let length: usize = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length: "))
            .unwrap()
            .parse()
            .unwrap();

        assert!(length < 8192);
        let mut body = vec![0; length];

        stream.read_exact(&mut body).unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();

        let request: rake::control::Request =
            serde_json::from_value(body["request"].clone()).unwrap();

        assert_eq!(request.protocol, rake::control::PROTOCOL_VERSION);
        assert_eq!(request.runtime_id, runtime);
        assert!(request.key.is_none());
        assert!(matches!(
            request.message,
            rake::control::Message::Query(Query::InspectTree {})
        ));
        let reply: Result<rake::control::Reply, rake::control::ControlError> =
            Ok(rake::control::Reply::Elements(rake::inspection::Snapshot {
                status: rake::inspection::Status::Captured,
                capture: "capture-1".into(),
                router_revision: 3,
                elements: vec![],
                omitted: 0,
                coverage: "boundaries".into(),
            }));

        let body = serde_json::to_vec(&reply).unwrap();

        write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n", body.len()).unwrap();
        stream.write_all(&body).unwrap();
    });

    let config = ConnectionConfig::from_launch_url(&format!(
        "http://{address}/__rake/inspector#__rake_control={}",
        "x".repeat(40)
    ))
    .unwrap();

    let connection = Connection::new(config).unwrap();

    let reply = futures::executor::block_on(connection.query(
        runtime,
        Query::InspectTree {},
        CancellationToken::new(),
    ))
    .unwrap();

    assert!(matches!(reply, rake::control::Reply::Elements(tree) if tree.capture == "capture-1"));
    server.join().unwrap();
}
