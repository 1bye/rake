//! Covers: Python bridge/broker queue, lease, discovery and forwarding contracts.
//! Real: bridge, files, sockets and HTTP. Doubles: runtime peers and source opener.
//! Requires: Unix and loopback sockets; no display.
use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

fn broker(root: &Path) -> Broker {
    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
    Broker::new(Options {
        url: "http://127.0.0.1:8081".into(),
        directory: root.into(),
        development: None,
        output: None,
        allow_native: true,
        cdp_url: None,
        source_root: root.into(),
        cli: "/nonexistent-rake".into(),
    })
    .unwrap()
}
fn connect(b: &Broker, runtime: &str) {
    assert_eq!(b.handle("connect", &json!({"application_id":"test.web", "info":{"protocol":1,"platform":"web","runtime_id":runtime}})).unwrap(), json!({"connected":true}));
}

#[test]
fn owned_protocol_roundtrip_redelivery_private_discovery_and_cleanup() {
    let root = tempfile::tempdir().unwrap();
    let b = broker(root.path());
    let runtime = uuid::Uuid::new_v4().to_string();
    connect(&b, &runtime);
    assert_eq!(
        fs::metadata(root.path().join(format!("{runtime}.json")))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let discovered = b.handle("inspect_instances", &json!({})).unwrap();
    assert!(discovered[0].get("endpoint").is_none());
    assert!(!discovered.to_string().contains(&b.bridge.token));
    let request = json!({"protocol":1,"runtime_id":runtime,"key":"same-retry-key","message":{"type":"query","value":{"type":"info"}}});
    std::thread::scope(|scope| {
        let client = scope.spawn(|| b.handle("inspect", &json!({"request":request})).unwrap());
        let delivery = b.handle("poll", &json!({"runtime_id":runtime})).unwrap();
        assert_eq!(delivery[0]["request"], request);
        assert_eq!(
            b.handle("poll", &json!({"runtime_id":runtime})).unwrap(),
            delivery
        );
        let result = json!({"Ok":{"type":"info","value":{}}});
        b.handle(
            "reply",
            &json!({"runtime_id":runtime,"ticket":delivery[0]["ticket"],"result":result}),
        )
        .unwrap();
        assert_eq!(client.join().unwrap(), result);
    });
    b.close();
    assert!(!root.path().join(format!("{runtime}.json")).exists());
}

#[test]
fn capacity_expired_leases_and_tickets_keep_timeout_delivery_semantics() {
    let root = tempfile::tempdir().unwrap();
    let b = broker(root.path());
    let runtime = uuid::Uuid::new_v4().to_string();
    connect(&b, &runtime);
    {
        let mut state = b.bridge.state.lock().unwrap();
        for _ in 0..32 {
            state.tickets.push(Ticket {
                id: uuid::Uuid::new_v4().simple().to_string(),
                runtime: runtime.clone(),
                request: json!({}),
                result: None,
                created: Instant::now(),
            });
        }
    }
    assert_eq!(
        b.handle(
            "client",
            &json!({"runtime_id":runtime,"request":{"runtime_id":runtime,"protocol":1}})
        )
        .unwrap_err()
        .status,
        429
    );
    {
        let mut state = b.bridge.state.lock().unwrap();
        state.instances.get_mut(&runtime).unwrap().seen -= Duration::from_secs(31);
        state.tickets[0].created -= Duration::from_secs(61);
    }
    assert_eq!(
        b.handle("poll", &json!({"runtime_id":runtime}))
            .unwrap_err()
            .status,
        404
    );
    assert!(!root.path().join(format!("{runtime}.json")).exists());
    assert_eq!(b.bridge.state.lock().unwrap().tickets.len(), 31);
}

#[test]
fn private_reads_reject_links_public_files_fifos_and_oversized_frames() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("output.json");
    fs::write(&path, "{}").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(private_read(&path).unwrap_err().status, 403);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(private_read(&path).unwrap(), json!({}));
    symlink(&path, root.path().join("link")).unwrap();
    assert!(private_read(&root.path().join("link")).is_err());
    fs::write(&path, vec![b' '; MAX_BYTES + 1]).unwrap();
    assert_eq!(private_read(&path).unwrap_err().status, 413);
    let fifo =
        std::ffi::CString::new(root.path().join("fifo").as_os_str().as_encoded_bytes()).unwrap();
    // SAFETY: fifo is a live NUL-terminated path; mkfifo retains no pointer.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    assert_eq!(
        private_read(&root.path().join("fifo")).unwrap_err().status,
        403
    );
}

#[test]
fn source_targets_accept_only_exact_element_ids_and_captured_indices() {
    for id in [
        "--session",
        "../app.rs",
        "element-1-0;touch",
        "",
        "element-é",
    ] {
        assert!(source::arguments(&json!({"id":id})).is_err(), "{id}");
    }
    assert_eq!(
        source::arguments(&json!({"id":"element-1-0"})).unwrap(),
        vec!["element-1-0"]
    );
    for kind in ["style", "handler"] {
        assert_eq!(
            source::arguments(&json!({"id":"element-1-0","target":{"kind":kind,"index":0}}))
                .unwrap(),
            vec!["element-1-0".to_owned(), format!("--{kind}"), "0".into()]
        );
    }
    for target in [
        json!({"kind":"style","index":-1}),
        json!({"kind":"style","index":true}),
        json!({"kind":"handler","index":"--help"}),
        json!({"kind":"boundary","file":"/secret.rs"}),
        json!({"kind":"other"}),
    ] {
        assert!(source::arguments(&json!({"id":"element-1-0","target":target})).is_err());
    }
}

#[test]
fn loopback_endpoints_reject_credentials_paths_aliases_and_redirect_targets() {
    for endpoint in [
        "https://example.test",
        "http://127.0.0.1:9/other",
        "http://user@127.0.0.1:9",
        "http://127.0.0.1:9/?q=1",
        "http://127.0.0.1:9/#x",
        "http://127.1:9",
        "http://[::1]:9",
    ] {
        assert!(endpoint_url(endpoint, false).is_err(), "{endpoint}");
    }
    assert!(endpoint_url("http://localhost:9", false).is_ok());
    assert!(endpoint_url("http://localhost:9", true).is_err());
    assert!(endpoint_url("http://127.0.0.1:80", true).is_ok());
}

#[test]
fn native_forwarding_keeps_the_exact_envelope_and_rejects_foreign_or_public_sockets() {
    use std::{
        io::{BufRead, BufReader, Write},
        os::unix::net::UnixListener,
    };
    let root = tempfile::tempdir().unwrap();
    let b = broker(root.path());
    let runtime = uuid::Uuid::new_v4().to_string();
    let path = root.path().join(format!("{runtime}.sock"));
    let listener = UnixListener::bind(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let manifest = json!({"application_id":"native", "pid":1, "info":{"runtime_id":runtime},"endpoint":{"type":"local","path":path}});
    super::super::write_marker(&root.path().join(format!("{runtime}.json")), &manifest).unwrap();
    let request = json!({"protocol":1,"runtime_id":runtime,"key":"same-retry-key","message":{"type":"command","value":{"type":"close","tab":7}},"extension":"retained"});
    let expected = request.clone();
    let peer = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut line = String::new();
        BufReader::new(&mut stream).read_line(&mut line).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap(), expected);
        stream
            .write_all(b"{\"Ok\":{\"type\":\"submitted\",\"value\":{\"operation_id\":9}}}\n")
            .unwrap();
    });
    assert_eq!(
        b.handle("inspect", &json!({"request":request})).unwrap()["Ok"]["value"]["operation_id"],
        9
    );
    peer.join().unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        b.handle("inspect", &json!({"request":request}))
            .unwrap_err()
            .status,
        403
    );
    let mut altered = manifest;
    altered["endpoint"]["path"] = json!(root.path().join("other.sock"));
    super::super::write_marker(&root.path().join(format!("{runtime}.json")), &altered).unwrap();
    assert_eq!(
        b.handle("inspect", &json!({"request":request}))
            .unwrap_err()
            .status,
        403
    );
    let mut web = broker(root.path());
    web.options.allow_native = false;
    assert_eq!(web.instances().unwrap(), json!([]));
    assert_eq!(
        web.handle("inspect", &json!({"request":request}))
            .unwrap_err()
            .status,
        403
    );
    assert_eq!(
        web.handle(
            "inspect_source",
            &json!({"runtime_id":runtime,"id":"element-1-0"})
        )
        .unwrap_err()
        .status,
        403
    );
}

#[test]
fn native_successor_metadata_belongs_only_to_the_supervisor_runtime() {
    let root = tempfile::tempdir().unwrap();
    let mut b = broker(root.path());
    let runtime = uuid::Uuid::new_v4().to_string();
    let other = uuid::Uuid::new_v4().to_string();
    let predecessor = uuid::Uuid::new_v4().to_string();
    let status = root.path().join("status");
    fs::write(
        &status,
        json!({"runtime_id":runtime,"reload":{"phase":"restored","restored_from":predecessor}})
            .to_string(),
    )
    .unwrap();
    b.bridge.development = Some(status);
    for id in [&runtime, &other] {
        let manifest = json!({"application_id":"native","pid":1,"info":{"runtime_id":id},"endpoint":{"type":"local","path":"unused"}});
        super::super::write_marker(&root.path().join(format!("{id}.json")), &manifest).unwrap();
    }
    let found = b.instances().unwrap();
    let found = found.as_array().unwrap();
    assert_eq!(
        found
            .iter()
            .find(|item| item["info"]["runtime_id"] == runtime)
            .unwrap()["restored_from"],
        predecessor
    );
    assert!(
        found
            .iter()
            .find(|item| item["info"]["runtime_id"] == other)
            .unwrap()["restored_from"]
            .is_null()
    );
}

#[test]
fn output_is_private_bounded_and_cursor_filtered_without_inventing_causal_ownership() {
    let root = tempfile::tempdir().unwrap();
    let mut b = broker(root.path());
    let path = root.path().join("output");
    b.options.output = Some(path.clone());
    assert_eq!(
        b.handle("inspect_output", &json!({})).unwrap(),
        json!({"records":[],"dropped":0})
    );
    super::super::write_marker(&path,&json!({"records":[{"sequence":1,"text":"first"},{"sequence":2,"text":"second"}],"dropped":22,"revision":2})).unwrap();
    let result = b.handle("inspect_output", &json!({"after":1})).unwrap();
    assert_eq!(result["records"], json!([{"sequence":2,"text":"second"}]));
    assert_eq!(result["dropped"], 22);
    for after in [json!(true), json!(-1), json!("1"), json!(null)] {
        assert_eq!(
            b.handle("inspect_output", &json!({"after":after}))
                .unwrap_err()
                .status,
            400
        );
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        b.handle("inspect_output", &json!({})).unwrap_err().status,
        403
    );
}

#[test]
fn source_opener_receives_exact_arguments_control_directory_and_source_root() {
    let root = tempfile::tempdir().unwrap();
    let mut b = broker(root.path());
    let runtime = uuid::Uuid::new_v4().to_string();
    connect(&b, &runtime);
    let tool = root.path().join("source-double");
    fs::write(&tool,"#!/bin/sh\n[ \"$1\" = ctl ] && [ \"$2\" = --instance ] && [ \"$4\" = inspect ] && [ \"$5\" = source ] && [ \"$6\" = element-1-0 ] && [ \"$7\" = --style ] && [ \"$8\" = 0 ] && [ \"$9\" = --json ] && [ \"$#\" = 9 ] && [ \"$RAKE_CONTROL_DIR\" = \"$RAKE_SOURCE_ROOT\" ] || exit 7\nprintf '{\"path\":\"/workspace/app.rs\",\"line\":3,\"column\":4}'\n").unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o700)).unwrap();
    b.options.cli = tool.clone();
    let body = json!({"runtime_id":runtime,"id":"element-1-0","target":{"kind":"style","index":0},"file":"/outside.rs"});
    assert_eq!(b.handle("inspect_source", &body).unwrap()["line"], 3);
    fs::write(
        &tool,
        "#!/bin/sh\nprintf 'Cannot open source' >&2\nexit 7\n",
    )
    .unwrap();
    assert_eq!(
        b.handle("inspect_source", &body).unwrap_err().to_string(),
        "Cannot open source"
    );
    fs::write(&tool, "#!/bin/sh\nprintf invalid\n").unwrap();
    assert_eq!(b.handle("inspect_source", &body).unwrap_err().status, 502);
}

#[test]
fn external_web_and_cdp_forwarding_reject_redirects_and_bound_response_bodies() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };
    let root = tempfile::tempdir().unwrap();
    let mut b = broker(root.path());
    assert_eq!(b.handle("inspect_cdp", &json!({})).unwrap(), json!([]));
    for oversized in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        b.options.cdp_url = Some(format!("http://{address}"));
        let peer = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                header.push(byte[0]);
            }
            assert!(header.starts_with(b"GET /json/list "));
            let response = if oversized {
                format!(
                    "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                    MAX_BYTES + 1,
                    " ".repeat(MAX_BYTES + 1)
                )
            } else {
                "HTTP/1.0 302 Found\r\nLocation: http://foreign.invalid/\r\nContent-Length: 0\r\n\r\n".into()
            };
            let _ = stream.write_all(response.as_bytes());
        });
        let error = b.handle("inspect_cdp", &json!({})).unwrap_err();
        assert_eq!(error.status, if oversized { 413 } else { 502 });
        if !oversized {
            assert_eq!(error.to_string(), "development endpoints cannot redirect");
        }
        peer.join().unwrap();
    }
}

#[test]
fn reload_busy_retries_and_shutdown_wakes_pending_clients() {
    let root = tempfile::tempdir().unwrap();
    let mut b = broker(root.path());
    let runtime = uuid::Uuid::new_v4().to_string();
    connect(&b, &runtime);
    let status = root.path().join("status");
    fs::write(&status, "{\"published\":2}").unwrap();
    b.bridge.development = Some(status);
    assert_eq!(
        b.handle("poll", &json!({"runtime_id":runtime})).unwrap(),
        json!([{"ticket":"reload-2","reload_build":2}])
    );
    b.handle(
        "reload_result",
        &json!({"runtime_id":runtime,"error":"runtime is busy"}),
    )
    .unwrap();
    b.bridge
        .state
        .lock()
        .unwrap()
        .instances
        .get_mut(&runtime)
        .unwrap()
        .retry_after = Instant::now();
    assert_eq!(
        b.handle("poll", &json!({"runtime_id":runtime})).unwrap()[0]["reload_build"],
        2
    );
    std::thread::scope(|scope| {
        let client = scope.spawn(|| {
            b.handle(
                "client",
                &json!({"runtime_id":runtime,"request":{"runtime_id":runtime,"protocol":1}}),
            )
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            assert!(
                Instant::now() < deadline,
                "pending client must enqueue before deadline"
            );
            if !b.bridge.state.lock().unwrap().tickets.is_empty() {
                break;
            }
            std::thread::yield_now();
        }
        b.close();
        assert_eq!(client.join().unwrap().unwrap_err().status, 503);
    });
}

#[test]
fn external_web_forwarding_preserves_credentials_and_envelope_only_at_validated_origin() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };
    let root = tempfile::tempdir().unwrap();
    let b = broker(root.path());
    let runtime = uuid::Uuid::new_v4().to_string();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let manifest = json!({"application_id":"external.web","pid":1,"info":{"runtime_id":runtime},"endpoint":{"type":"web","url":format!("http://{address}"),"token":"private-external-token"}});
    super::super::write_marker(&root.path().join(format!("{runtime}.json")), &manifest).unwrap();
    let request = json!({"protocol":1,"runtime_id":runtime,"key":"unchanged","message":{"type":"query","value":{"type":"snapshot"}}});
    let expected = json!({"runtime_id":runtime,"request":request});
    let peer = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            header.push(byte[0]);
        }
        let header = String::from_utf8(header).unwrap().to_lowercase();
        assert!(header.starts_with("post /__rake/client "));
        assert!(header.contains("x-rake-token: private-external-token\r\n"));
        let length: usize = header
            .lines()
            .find_map(|line| line.strip_prefix("content-length: "))
            .unwrap()
            .parse()
            .unwrap();
        let mut bytes = vec![0; length];
        stream.read_exact(&mut bytes).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), expected);
        stream
            .write_all(b"HTTP/1.0 200 OK\r\nContent-Length: 9\r\n\r\n{\"Ok\":{}}")
            .unwrap();
    });
    assert_eq!(
        b.handle("inspect", &json!({"request":request})).unwrap(),
        json!({"Ok":{}})
    );
    peer.join().unwrap();
}

#[test]
fn source_output_overflow_is_bounded_and_stopped_brokers_do_not_launch_openers() {
    let root = tempfile::tempdir().unwrap();
    let mut b = broker(root.path());
    let runtime = uuid::Uuid::new_v4().to_string();
    connect(&b, &runtime);
    let tool = root.path().join("source-double");
    fs::write(
        &tool,
        "#!/bin/sh\nexec /bin/dd if=/dev/zero bs=1048576 count=2 2>/dev/null\n",
    )
    .unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o700)).unwrap();
    b.options.cli = tool;
    let body = json!({"runtime_id":runtime,"id":"element-1-0"});
    assert_eq!(b.handle("inspect_source", &body).unwrap_err().status, 502);
    b.cancelled.cancel();
    assert_eq!(
        source::open(&b.options, &runtime, &["element-1-0".into()], &b.cancelled)
            .unwrap_err()
            .status,
        502
    );
}

#[test]
fn native_reply_bounds_and_shutdown_cancel_pending_socket_work() {
    use std::{
        io::{BufRead, BufReader, Read, Write},
        os::unix::net::UnixListener,
        sync::mpsc,
    };
    for oversized in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let b = broker(root.path());
        let runtime = uuid::Uuid::new_v4().to_string();
        let path = root.path().join(format!("{runtime}.sock"));
        let listener = UnixListener::bind(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        super::super::write_marker(&root.path().join(format!("{runtime}.json")),&json!({"application_id":"native","pid":1,"info":{"runtime_id":runtime},"endpoint":{"type":"local","path":path}})).unwrap();
        let (sent, received) = mpsc::sync_channel(1);
        std::thread::scope(|scope| {
            let peer = scope.spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = String::new();
                BufReader::new(&mut stream).read_line(&mut request).unwrap();
                sent.send(()).unwrap();
                if oversized {
                    let _ = stream.write_all(&vec![b' '; MAX_BYTES + 1]);
                } else {
                    assert_eq!(
                        stream.read(&mut [0]).unwrap(),
                        0,
                        "shutdown must close the owned peer"
                    );
                }
            });
            let client=scope.spawn(||b.handle("inspect",&json!({"request":{"protocol":1,"runtime_id":runtime,"key":null,"message":{"type":"query","value":{"type":"info"}}}})));
            received.recv_timeout(Duration::from_secs(3)).unwrap();
            if !oversized {
                b.close();
            }
            assert_eq!(
                client.join().unwrap().unwrap_err().status,
                if oversized { 413 } else { 503 }
            );
            peer.join().unwrap();
        });
    }
}
