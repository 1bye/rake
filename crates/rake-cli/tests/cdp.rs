//! Covers: CDP discovery, access checks, wire replies and runtime detachment.
//! Real: CLI process, TCP/WebSocket, private Unix IPC, headless router.
//! Doubles: no GUI. Requires: Unix sockets and loopback listeners.

#![cfg(unix)]

use futures::channel::oneshot;
use futures::future::select;
use rake::control::local::LocalServer;
use rake::router::{NoRoute, Router, RouterConfig};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

struct Adapter(Child);

impl Drop for Adapter {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn http(authority: &str, path: &str, host: &str, origin: Option<&str>) -> String {
    let mut stream = TcpStream::connect(authority).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    write!(stream, "GET {path} HTTP/1.1\r\nHost: {host}\r\n").unwrap();

    if let Some(origin) = origin {
        write!(stream, "Origin: {origin}\r\n").unwrap();
    }

    write!(stream, "\r\n").unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).unwrap();

    reply
}

#[test]
fn discovery_access_wire_and_detach() {
    let directory = std::path::PathBuf::from(format!("/tmp/rake-cdp-{}", uuid::Uuid::new_v4()));
    let router = Router::<NoRoute>::new(RouterConfig::default());
    let client = router.client();
    let server = LocalServer::bind(&client, "test.cdp", &directory).unwrap();
    let (gone, vanish) = oneshot::channel();
    let worker_directory = directory.clone();
    let worker = std::thread::spawn(move || {
        let mut adapter = Adapter(
            Command::new(env!("CARGO_BIN_EXE_rake"))
                .env("RAKE_CONTROL_DIR", &worker_directory)
                .args(["cdp", "--port", "0", "--no-open", "--json"])
                .stdout(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let mut line = String::new();
        BufReader::new(adapter.0.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let ready: Value = serde_json::from_str(&line).unwrap();
        let authority = ready["url"]
            .as_str()
            .unwrap()
            .strip_prefix("http://")
            .unwrap();
        assert!(ready["port"].as_u64().unwrap() > 0);
        assert_eq!(ready["targets"][0]["type"], "page");
        assert_eq!(ready["targets"][0]["title"], "test.cdp");
        assert!(http(authority, "/json/list", "evil.test", None).starts_with("HTTP/1.1 403"));
        assert!(
            http(
                authority,
                "/json/list",
                authority,
                Some("https://evil.test")
            )
            .starts_with("HTTP/1.1 403")
        );
        assert!(http(authority, "/bad-token/runtime", authority, None).starts_with("HTTP/1.1 403"));
        let list = http(
            authority,
            "/json/list",
            authority,
            Some("devtools://devtools"),
        );
        let list: Value = serde_json::from_str(list.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(list, ready["targets"]);
        let (mut socket, _) =
            tungstenite::connect(list[0]["webSocketDebuggerUrl"].as_str().unwrap()).unwrap();

        if let tungstenite::stream::MaybeTlsStream::Plain(stream) = socket.get_mut() {
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
        }

        socket
            .send(tungstenite::Message::text(
                json!({"id":1,"method":"Unknown.method"}).to_string(),
            ))
            .unwrap();
        let reply: Value = serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(reply["id"], 1);
        assert_eq!(reply["error"]["code"], -32601);
        socket
            .send(tungstenite::Message::text(
                json!({"id":2,"method":"Network.enable"}).to_string(),
            ))
            .unwrap();
        let reply: Value = serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(reply["result"], json!({}));
        gone.send(()).unwrap();
        let reply: Value = serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(reply["method"], "Inspector.detached");
    });

    futures::executor::block_on(async {
        let work = async {
            let _ = select(Box::pin(router.run()), Box::pin(server.run(client))).await;
        };
        let _ = select(Box::pin(work), Box::pin(vanish)).await;
    });
    drop(server);
    worker.join().unwrap();
    router.shutdown();
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn real_http_lifecycle_crosses_protocol_and_websocket() {
    // A diagnostics-only protocol fixture shares the real HTTP collector. The other
    // integration test covers live router discovery and identity handling.
    use rake::control::{Reply, Request};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    let directory = std::path::PathBuf::from(format!("/tmp/rake-cdp-{}", uuid::Uuid::new_v4()));
    let router = Router::<NoRoute>::new(RouterConfig::default());
    let server = LocalServer::bind(&router.client(), "test.http", &directory).unwrap();
    let instance = rake::control::local::discover(&directory)
        .unwrap()
        .remove(0);
    let rake::control::local::Endpoint::Local { path } = &instance.endpoint else {
        panic!()
    };
    std::fs::remove_file(path).unwrap();
    let listener = UnixListener::bind(path).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let collector = rake::diagnostics::Collector::new(32);
    let fixture_collector = collector.clone();
    let info = instance.info.clone();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let fixture_stop = stop.clone();
    let fixture = std::thread::spawn(move || {
        while !fixture_stop.load(std::sync::atomic::Ordering::Relaxed) {
            let (mut socket, _) = match listener.accept() {
                Ok(value) => value,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(2));
                    continue;
                }
                Err(e) => panic!("{e}"),
            };
            let mut line = String::new();
            BufReader::new(socket.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let request: Request = serde_json::from_str(&line).unwrap();
            let reply = match request.message {
                rake::control::Message::Query(rake::control::Query::Diagnostics {
                    after, ..
                }) => {
                    let mut snapshot = fixture_collector.snapshot();
                    snapshot.records.retain(|r| r.sequence > after);
                    Reply::Diagnostics(snapshot)
                }
                _ => Reply::Info(info.clone()),
            };
            writeln!(
                socket,
                "{}",
                serde_json::to_string(&Ok::<_, rake::control::ControlError>(reply)).unwrap()
            )
            .unwrap();
        }
    });
    let mut adapter = Adapter(
        Command::new(env!("CARGO_BIN_EXE_rake"))
            .env("RAKE_CONTROL_DIR", &directory)
            .args(["cdp", "--no-open", "--json"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut line = String::new();
    BufReader::new(adapter.0.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let ready: Value = serde_json::from_str(&line).unwrap();
    let (mut socket, _) = tungstenite::connect(
        ready["targets"][0]["webSocketDebuggerUrl"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    if let tungstenite::stream::MaybeTlsStream::Plain(stream) = socket.get_mut() {
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
    }
    socket
        .send(tungstenite::Message::text(
            json!({"id":1,"method":"Network.enable"}).to_string(),
        ))
        .unwrap();
    socket.read().unwrap();
    let http = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/test?secret=redacted", http.local_addr().unwrap());
    let responder = std::thread::spawn(move || {
        let (mut socket, _) = http.accept().unwrap();
        let mut request = [0; 4096];
        assert!(socket.read(&mut request).unwrap() > 0);
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ntest")
            .unwrap();
    });
    futures::executor::block_on(async {
        let http = rake::http::HttpClient::with_collector(rake::http::HttpConfig::new(), collector)
            .unwrap();
        let body = http.get(&url).send().await.unwrap().bytes().await.unwrap();
        assert_eq!(body.as_slice(), b"test");
    });
    responder.join().unwrap();
    let mut methods = Vec::new();
    loop {
        let event: Value = serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
        let method = event["method"].as_str().unwrap().to_owned();
        methods.push(method.clone());
        if method == "Network.loadingFinished" {
            assert_eq!(event["params"]["encodedDataLength"], 4);
            break;
        }
    }
    assert_eq!(
        methods,
        [
            "Network.requestWillBeSent",
            "Network.responseReceived",
            "Network.dataReceived",
            "Network.loadingFinished"
        ]
    );
    drop(socket);
    drop(adapter);
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    fixture.join().unwrap();
    drop(server);
    std::fs::remove_dir_all(directory).unwrap();
}
