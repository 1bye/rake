//! Covers: busy runtime responsiveness, ordered clients/reconnects, and slow-reader shutdown.
//! Real: TCP/WebSocket transport, bounded worker channels, deadlines and joined threads.
//! Doubles: deterministic session workers. Requires: ephemeral loopback TCP sockets.

use super::*;

#[test]
fn disconnect_during_receive_cancels_queued_mutations_and_idle_polling() {
    for queued in [true, false] {
        let gates = Arc::new((std::sync::Barrier::new(2), std::sync::Barrier::new(2)));
        let (input, requests) = mpsc::sync_channel(4);
        let (events, _output) = mpsc::sync_channel(2);
        let closed = Arc::new(AtomicBool::new(false));
        let worker = Worker {
            input: requests,
            output: events,
            closed: Arc::clone(&closed),
            before_receive: Some(Arc::clone(&gates)),
        };
        let reader = std::thread::spawn(move || worker.read());

        gates.0.wait();
        if queued {
            input
                .send(Work {
                    request: Request {
                        id: 1,
                        method: "DOM.setNodeValue".into(),
                        params: serde_json::json!({"nodeId":2,"value":"changed"}),
                    },
                    deadline: Instant::now() + Duration::from_secs(2),
                })
                .unwrap();
        }
        closed.store(true, Ordering::Release);
        gates.1.wait();

        assert!(
            matches!(
                reader.join().unwrap(),
                Err(mpsc::RecvTimeoutError::Disconnected)
            ),
            "disconnect must stop both queued mutations and polling after an idle receive"
        );
    }
}

#[test]
fn busy_worker_preserves_ping_startup_ack_and_bounded_admission() {
    for (params, last_id) in [
        (serde_json::Value::Null, REQUEST_LIMIT as u64 + 2),
        (serde_json::json!({"data":"x".repeat(512 * 1024)}), 10),
    ] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (started, ready) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve(tungstenite::accept(stream).unwrap(), move |worker| {
                let first = worker.read().unwrap();
                started.send(()).unwrap();
                blocked.recv_timeout(Duration::from_secs(5)).unwrap();
                let _ = worker.reply(Response::new(
                    Some(first.request.id),
                    Ok(serde_json::json!({})),
                ));
                assert!(worker.read().is_err(), "disconnect cancels queued requests");
                Ok(())
            })
            .unwrap();
        });
        let stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let (mut client, _) = tungstenite::client(format!("ws://{address}"), stream).unwrap();
        client
            .send(Message::text(r#"{"id":1,"method":"DOM.getDocument"}"#))
            .unwrap();
        ready.recv_timeout(Duration::from_secs(2)).unwrap();
        client
            .send(Message::text(r#"{"id":2,"method":"Inspector.enable"}"#))
            .unwrap();
        let response: serde_json::Value =
            serde_json::from_str(client.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(
            response["id"], 2,
            "startup acknowledgement must not wait for the worker"
        );
        client.send(Message::Ping(vec![1, 2].into())).unwrap();
        assert!(matches!(client.read().unwrap(), Message::Pong(_)));
        for id in 3..=last_id {
            client
                .send(Message::text(
                    serde_json::json!({"id":id,"method":"DOM.getDocument","params":params})
                        .to_string(),
                ))
                .unwrap();
        }
        let response: serde_json::Value =
            serde_json::from_str(client.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(response["id"], last_id);
        assert_eq!(response["error"]["code"], -32000);
        client.close(None).unwrap();
        assert!(matches!(client.read().unwrap(), Message::Close(_)));
        release.send(()).unwrap();
        server.join().unwrap();
    }
}

#[test]
fn chrome_startup_burst_reaches_runtime_enable_before_worker_can_drain() {
    // Recorded from stock Chrome 156: its first burst used to reject Runtime.enable.
    let methods = [
        "Network.enable",
        "Network.configureDurableMessages",
        "Network.setAttachDebugStack",
        "Page.enable",
        "Page.getResourceTree",
        "Profiler.enable",
        "Runtime.enable",
        "Debugger.enable",
        "Debugger.setPauseOnExceptions",
        "Debugger.setAsyncCallStackDepth",
        "Log.enable",
        "Log.startViolationsReport",
        "DOM.enable",
        "CSS.enable",
        "Overlay.enable",
        "Overlay.setShowViewportSizeOnResize",
        "Emulation.setEmulatedMedia",
        "Emulation.setEmulatedVisionDeficiency",
        "Animation.enable",
        "Autofill.enable",
        "Autofill.setAddresses",
        "Audits.enable",
        "ServiceWorker.enable",
        "Target.setAutoAttach",
        "Target.setDiscoverTargets",
        "Target.setRemoteLocations",
        "Runtime.addBinding",
        "Network.setBlockedURLs",
        "Network.emulateNetworkConditionsByRule",
        "Network.overrideNetworkState",
        "Runtime.getIsolateId",
        "Debugger.setBlackboxPatterns",
        "DOMDebugger.setBreakOnCSPViolation",
        "Page.setAdBlockingEnabled",
        "Emulation.setFocusEmulationEnabled",
        "Overlay.setShowGridOverlays",
        "Overlay.setShowFlexOverlays",
        "Overlay.setShowScrollSnapOverlays",
        "Overlay.setShowContainerQueryOverlays",
        "Overlay.setShowIsolatedElements",
        "Page.addScriptToEvaluateOnNewDocument",
    ];
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (release, blocked) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        serve(tungstenite::accept(stream).unwrap(), move |worker| {
            blocked.recv_timeout(Duration::from_secs(5)).unwrap();
            for method in methods {
                let request = worker.read().unwrap().request;
                assert_eq!(request.method, method);
                worker.reply(Response::new(Some(request.id), Ok(serde_json::json!({}))))?;
            }
            Ok(())
        })
        .unwrap();
    });
    let stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let (mut client, _) = tungstenite::client(format!("ws://{address}"), stream).unwrap();
    for (index, method) in methods.iter().enumerate() {
        client
            .send(Message::text(
                serde_json::json!({"id":index,"method":method,"params":{}}).to_string(),
            ))
            .unwrap();
    }
    // This stateless acknowledgement proves the owner admitted every preceding frame.
    client
        .send(Message::text(r#"{"id":999,"method":"Inspector.enable"}"#))
        .unwrap();
    let response: serde_json::Value =
        serde_json::from_str(client.read().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(
        response["id"], 999,
        "Chrome startup must not hit admission errors"
    );
    release.send(()).unwrap();
    for index in 0..methods.len() {
        let response: serde_json::Value =
            serde_json::from_str(client.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(response["id"], index);
        assert!(response.get("error").is_none());
    }
    server.join().unwrap();
}

#[test]
fn concurrent_clients_reconnect_and_receive_the_final_buffered_reply_in_order() {
    const CLIENTS: usize = 8;
    const REQUESTS: u64 = 16;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let mut connections = Vec::new();
        for _ in 0..CLIENTS * 2 {
            let (stream, _) = listener.accept().unwrap();
            connections.push(std::thread::spawn(move || {
                serve(tungstenite::accept(stream).unwrap(), |worker| {
                    for sequence in 1..=REQUESTS {
                        let work = worker.read()?;
                        assert_eq!(work.request.id, sequence);
                        worker.reply(Response::new(
                            Some(work.request.id),
                            Ok(serde_json::json!({"sequence":sequence})),
                        ))?;
                    }
                    Ok(())
                })
                .unwrap();
            }));
        }
        for connection in connections {
            connection.join().unwrap();
        }
    });
    let clients: Vec<_> = (0..CLIENTS)
        .map(|_| {
            std::thread::spawn(move || {
                for _ in 0..2 {
                    let stream = TcpStream::connect(address).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    let (mut client, _) =
                        tungstenite::client(format!("ws://{address}"), stream).unwrap();
                    for id in 1..=REQUESTS {
                        client
                            .send(Message::text(
                                serde_json::json!({"id":id,"method":"DOM.getDocument"}).to_string(),
                            ))
                            .unwrap();
                        let response: serde_json::Value =
                            serde_json::from_str(client.read().unwrap().to_text().unwrap())
                                .unwrap();
                        assert_eq!(response["id"], id);
                        assert_eq!(response["result"]["sequence"], id);
                    }
                }
            })
        })
        .collect();
    for client in clients {
        client.join().unwrap();
    }
    server.join().unwrap();
}

#[test]
fn nonreading_peer_is_disconnected_and_backpressured_worker_is_joined() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (done, finished) = mpsc::channel();
    let sent = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = Arc::clone(&sent);
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let socket = tungstenite::accept_with_config(
            stream,
            Some(
                tungstenite::protocol::WebSocketConfig::default()
                    .max_write_buffer_size(2 * 1024 * 1024),
            ),
        )
        .unwrap();
        let result = serve(socket, move |worker| {
            worker.read()?;
            let text = "x".repeat(256 * 1024);
            for _ in 0..1024 {
                if worker.send(Message::text(text.clone())).is_err() {
                    break;
                }
                observed.fetch_add(1, Ordering::Relaxed);
            }
            Ok(())
        });
        done.send(result.is_err()).unwrap();
    });
    let stream = TcpStream::connect(address).unwrap();
    let (mut client, _) = tungstenite::client(format!("ws://{address}"), stream).unwrap();
    client
        .send(Message::text(r#"{"id":1,"method":"DOM.getDocument"}"#))
        .unwrap();
    assert!(
        finished.recv_timeout(Duration::from_secs(5)).unwrap(),
        "write timeout must disconnect the stalled reader"
    );
    assert!(
        sent.load(Ordering::Relaxed) < 1024,
        "output must apply backpressure"
    );
    drop(client);
    server.join().unwrap();
}
