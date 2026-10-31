//! Covers: busy runtime responsiveness, ordered clients/reconnects, and slow-reader shutdown.
//! Real: TCP/WebSocket transport, bounded worker channels, deadlines and joined threads.
//! Doubles: deterministic session workers. Requires: ephemeral loopback TCP sockets.

use super::*;

#[test]
fn busy_worker_preserves_ping_startup_ack_and_bounded_admission() {
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
    for id in 3..=7 {
        client
            .send(Message::text(
                serde_json::json!({"id":id,"method":"DOM.getDocument"}).to_string(),
            ))
            .unwrap();
    }
    let response: serde_json::Value =
        serde_json::from_str(client.read().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(response["id"], 7);
    assert_eq!(response["error"]["code"], -32000);
    client.close(None).unwrap();
    assert!(matches!(client.read().unwrap(), Message::Close(_)));
    release.send(()).unwrap();
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
