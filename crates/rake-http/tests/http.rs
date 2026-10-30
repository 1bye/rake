//! Covers: real HTTP/JSON, raw statuses, streaming, deadlines and cancellation.
//! Real: Rake transport, native executor adapter, TCP loopback server.
//! Doubles: deterministic server responses.
//! Requires: permission to bind an ephemeral loopback port; no external services.

#![cfg(not(target_family = "wasm"))]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use futures::executor::block_on;
use rake_diagnostics::Collector;
use rake_http::{CancellationToken, Error, HttpClient, HttpConfig};

fn server(response: impl Into<String>) -> (String, thread::JoinHandle<String>) {
    let response = response.into();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let url = format!("http://{}/api/", listener.local_addr().expect("address"));

    let thread = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let request = read_request(&mut stream);

        stream.write_all(response.as_bytes()).expect("respond");

        request
    });

    (url, thread)
}

fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("timeout");
    let mut bytes = Vec::new();
    let mut chunk = [0; 1024];

    loop {
        let count = stream.read(&mut chunk).expect("request bytes");

        assert_ne!(count, 0, "request ended before its body");
        bytes.extend_from_slice(&chunk[..count]);

        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]);

            let length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .and_then(|value| value.parse::<usize>().ok())
                })
                .unwrap_or(0);

            if bytes.len() >= end + 4 + length {
                break;
            }
        }
    }

    String::from_utf8(bytes).expect("UTF-8 fixture")
}

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct Document {
    title: String,
}

#[test]
fn json_query_and_body_work_without_a_tokio_executor() {
    let (base, server) = server(
        "HTTP/1.1 200 OK\r\nContent-Length: 14\r\nConnection: close\r\n\r\n{\"title\":\"ok\"}",
    );

    let client = HttpClient::new(HttpConfig::new().base_url(base)).expect("client");

    let draft = Document {
        title: "draft".into(),
    };

    let result: Document = block_on(
        client
            .post("documents")
            .query(&[("q", "a b&c")])
            .json(&draft)
            .send_json(),
    )
    .expect("JSON");

    let request = server.join().expect("server");

    assert_eq!(result.title, "ok");
    assert!(request.starts_with("POST /api/documents?q=a+b%26c HTTP/1.1"));
    assert!(request.contains("application/json"));
    assert!(request.ends_with("{\"title\":\"draft\"}"));
}

#[test]
fn raw_responses_preserve_error_status_and_body() {
    let (base, server) =
        server("HTTP/1.1 409 Conflict\r\nContent-Length: 8\r\nConnection: close\r\n\r\nconflict");

    let client = HttpClient::new(HttpConfig::new().base_url(base)).expect("client");

    let response = block_on(client.get("/documents").send()).expect("response");

    assert_eq!(response.status(), 409);
    let body = block_on(response.bytes()).expect("body");

    assert_eq!(body, b"conflict");
    assert!(
        server
            .join()
            .expect("server")
            .starts_with("GET /documents HTTP/1.1")
    );
}

#[test]
fn json_helpers_distinguish_status_and_decode_failures() {
    for (status, body, expected_status) in
        [("404 Not Found", "{}", Some(404)), ("200 OK", "no", None)]
    {
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );

        let (base, server) = server(response);
        let client = HttpClient::new(HttpConfig::new().base_url(base)).expect("client");

        let error = block_on(client.get("documents").send_json::<Document>()).expect_err("failure");

        match expected_status {
            Some(status) => {
                assert!(matches!(error, Error::Status { status: actual } if actual == status))
            }
            None => assert!(matches!(error, Error::Decode(_))),
        }

        server.join().expect("server");
    }
}

#[test]
fn invalid_configuration_and_pre_cancelled_requests_fail_without_io() {
    assert!(matches!(
        HttpClient::new(HttpConfig::new().base_url("file:///tmp/")),
        Err(Error::Configuration(_))
    ));
    let client = HttpClient::new(HttpConfig::new()).expect("client");
    let missing_base = block_on(client.get("documents").send());
    let token = CancellationToken::new();

    token.cancel();
    let cancelled = block_on(client.get("http://127.0.0.1:1/").cancellation(token).send());

    let timeout = block_on(
        client
            .get("http://127.0.0.1:1/")
            .timeout(Duration::ZERO)
            .send(),
    );

    assert!(matches!(missing_base, Err(Error::MissingBaseUrl)));
    assert!(matches!(cancelled, Err(Error::Cancelled)));
    assert!(matches!(timeout, Err(Error::Timeout)));
}

#[test]
fn cancellation_and_deadline_cover_a_pending_response_body() {
    for cancel in [true, false] {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let url = format!("http://{}/", listener.local_addr().expect("address"));
        let (release, wait) = mpsc::channel();

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");

            read_request(&mut stream);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n")
                .expect("headers");
            wait.recv_timeout(Duration::from_secs(3)).expect("release");
        });

        let token = CancellationToken::new();
        let client = HttpClient::new(HttpConfig::new()).expect("client");
        let request = client.get(&url).cancellation(token.clone());

        let request = if cancel {
            request
        } else {
            request.timeout(Duration::from_millis(250))
        };

        let mut response = block_on(request.send()).expect("headers");

        if cancel {
            token.cancel();
        }

        let error = block_on(response.next_chunk()).expect_err("body interrupted");

        release.send(()).expect("release server");
        server.join().expect("server");

        assert!(if cancel {
            matches!(error, Error::Cancelled)
        } else {
            matches!(error, Error::Timeout)
        });
    }
}

#[test]
fn capture_redacts_credentials_and_distinguishes_abandoned_bodies() {
    let (base, server) =
        server("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}");

    let collector = Collector::new(20);
    let client = HttpClient::with_collector(HttpConfig::new(), collector.clone()).expect("client");
    let url = base.replace("http://", "http://user:password@") + "items?token=secret";

    let response = block_on(
        client
            .get(&url)
            .header("authorization", "Bearer hidden")
            .send(),
    )
    .expect("headers");

    drop(response);
    server.join().expect("server");
    let snapshot = collector.snapshot();
    let encoded = serde_json::to_string(&snapshot).expect("snapshot");

    for secret in ["user", "password", "secret", "hidden"] {
        assert!(!encoded.contains(secret), "leaked {secret}");
    }

    assert!(encoded.contains("headers_received"));
    assert!(encoded.contains("abandoned"));
    assert!(!encoded.contains("completed"));
}

#[test]
fn network_rejection_and_truncated_bodies_have_distinct_errors() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("unused address");
    let endpoint = format!("http://{}/", listener.local_addr().expect("address"));

    drop(listener);
    let client =
        HttpClient::new(HttpConfig::new().timeout(Duration::from_secs(3))).expect("client");

    let rejected = block_on(client.get(&endpoint).send());

    assert!(matches!(rejected, Err(Error::Transport(_))));

    let (base, server) =
        server("HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\npartial");

    let response = block_on(client.get(&base).send()).expect("headers");
    let broken = block_on(response.bytes());

    server.join().expect("server");

    assert!(matches!(broken, Err(Error::Body(_))));
}

#[test]
fn request_body_abandonment_keeps_original_task_context_and_redacted_metadata() {
    use tracing::Instrument;
    use tracing_subscriber::prelude::*;

    let (base, server) =
        server("HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbody");

    let collector = Collector::new(50);
    let http = HttpClient::with_collector(HttpConfig::new(), collector.clone()).unwrap();

    let subscriber = tracing_subscriber::registry().with(collector.layer().with_filter(
        tracing_subscriber::filter::filter_fn(|metadata| metadata.name() != "http.request"),
    ));

    tracing::subscriber::with_default(subscriber, || {
        let original = tracing::info_span!(
            "original",
            task_id = "task-1",
            tab_id = "tab-7",
            generation_id = "gen-1"
        );

        let response = block_on(
            http.get(&format!("{base}documents?credential=secret#private"))
                .send()
                .instrument(original),
        )
        .unwrap();

        let unrelated = tracing::info_span!("unrelated", task_id = "task-2", tab_id = "tab-8");

        unrelated.in_scope(|| drop(response));
    });
    server.join().unwrap();
    let snapshot = collector.snapshot();

    let request = snapshot
        .records
        .iter()
        .find(|record| record.kind == "request" && record.fields["phase"] == "abandoned")
        .unwrap();

    assert_eq!(request.fields["task_id"], "task-1");
    assert_eq!(request.fields["tab_id"], "tab-7");
    assert_eq!(request.fields["method"], "GET");
    assert_eq!(request.fields["status"], 200);
    assert_eq!(request.fields["received_bytes"], 0);
    assert!(request.span_id.is_some());
    #[cfg(not(feature = "devtools"))]
    assert!(
        !request.fields["url"]
            .as_str()
            .unwrap()
            .contains("credential")
    );
    assert!(!request.fields["url"].as_str().unwrap().contains("secret"));
    assert!(!request.fields["url"].as_str().unwrap().contains("private"));
    assert!(request.fields["duration_us"].is_u64());
}

#[test]
fn overflowing_timers_stay_pending_without_panicking() {
    use futures::FutureExt;
    assert!(rake_http::sleep(Duration::MAX).now_or_never().is_none());
}

#[cfg(feature = "devtools")]
#[test]
fn inspection_captures_headers_and_query_without_credentials() {
    let (base, server) = server(
        "HTTP/1.1 201 Created\r\nContent-Length: 2\r\nContent-Type: application/json; charset=utf-8\r\nX-Reply: one\r\nX-Reply: two\r\nSet-Cookie: private=value\r\nConnection: close\r\n\r\n{}",
    );
    let collector = Collector::default();
    let client = HttpClient::with_collector(HttpConfig::new(), collector.clone()).unwrap();
    block_on(
        client
            .post(&base)
            .query(&[("q", "a b"), ("token", "secret")])
            .header("authorization", "Bearer secret")
            .header("x-fixture", "visible")
            .json(&serde_json::json!({"title":"draft"}))
            .send(),
    )
    .unwrap();
    let request = server.join().unwrap();
    assert!(
        request.contains("token=secret"),
        "capture must not modify transmission"
    );
    let records = collector.snapshot_after(0, Some("request")).records;
    let started = &records[0].fields;
    assert!(
        started["url"]
            .as_str()
            .unwrap()
            .contains("q=a+b&token=%5Bredacted%5D")
    );
    assert_eq!(started["request_headers"]["authorization"], "[redacted]");
    assert_eq!(started["request_headers"]["x-fixture"], "visible");
    let response = records
        .iter()
        .find(|r| r.fields["phase"] == "headers_received")
        .unwrap();
    assert_eq!(response.fields["response_headers"]["x-reply"], "one\ntwo");
    assert_eq!(
        response.fields["response_headers"]["set-cookie"],
        "[redacted]"
    );
    assert_eq!(response.fields["status_text"], "Created");
    assert!(!serde_json::to_string(&records).unwrap().contains("secret"));
}
