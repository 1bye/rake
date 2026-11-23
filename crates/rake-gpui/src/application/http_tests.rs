//! Covers: the GPUI HTTP adapter's method, headers, bodies and raw status forwarding.
//! Real: Rake transport and TCP loopback. Doubles: one deterministic HTTP response.
//! Requires: permission to bind a loopback port; no display or external services.

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::time::Duration;

use futures::{AsyncReadExt as _, executor::block_on};
use gpui::http_client::{AsyncBody, HttpClient as _, Request};

use super::{GpuiHttp, HttpClient, HttpConfig};

#[test]
fn forwards_request_and_preserves_non_success_response() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture");

    let url = format!(
        "http://{}/image",
        listener.local_addr().expect("fixture address")
    );

    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().expect("accept request");

        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .expect("read timeout");
        let mut request = Vec::new();
        let mut buffer = [0; 1024];

        while !request.ends_with(b"payload") {
            let count = socket.read(&mut buffer).expect("read request");

            assert_ne!(count, 0, "request ended before body");
            request.extend_from_slice(&buffer[..count]);
        }

        socket.write_all(b"HTTP/1.1 409 Conflict\r\nX-Fixture: retained\r\nContent-Length: 8\r\nConnection: close\r\n\r\nconflict").expect("fixture response");

        String::from_utf8(request).expect("text fixture")
    });

    let follow = HttpClient::new(HttpConfig::new()).expect("client");

    let adapter = GpuiHttp {
        manual: follow.without_redirects().expect("manual client"),
        follow,
    };

    let request = Request::builder()
        .method("POST")
        .uri(url)
        .header("x-request", "retained")
        .body(AsyncBody::from("payload"))
        .expect("GPUI request");

    let mut response = block_on(adapter.send(request)).expect("GPUI response");
    let mut body = String::new();

    block_on(response.body_mut().read_to_string(&mut body)).expect("GPUI response body");
    let request = server.join().expect("fixture server");

    assert_eq!(response.status(), 409);
    assert_eq!(response.headers()["x-fixture"], "retained");
    assert_eq!(body, "conflict");
    assert!(request.starts_with("POST /image HTTP/1.1"), "{request}");
    assert!(request.contains("x-request: retained"), "{request}");
    assert!(request.ends_with("payload"), "{request}");
}
