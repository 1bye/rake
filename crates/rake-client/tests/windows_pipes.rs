//! Real Windows named pipes: finite exchanges, identity and bounded responses.
#![cfg(windows)]

use futures::{
    future::{Either, select},
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
};

use rake_client::{
    local,
    pipe::{self, Listener, Stream},
};

use rake_protocol::control::{
    ControlError, MAX_MESSAGE_BYTES, Message, PROTOCOL_VERSION, Query, Reply, Request,
};

use std::{
    io,
    time::{Duration, Instant},
};

fn identity() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}

fn request() -> Request {
    serde_json::from_value(serde_json::json!({"protocol":PROTOCOL_VERSION, "runtime_id":"00000000-0000-4000-8000-000000000001", "key":null, "message":{"type":"query","value":{"type":"info"}}})).unwrap()
}

#[test]
fn pipe_round_trip_and_oversized_reply_are_bounded() {
    for oversized in [false, true] {
        let path = pipe::path(&identity());
        let listener = Listener::bind(&path).unwrap();

        assert!(
            Listener::bind(&path).is_err(),
            "duplicate listener accepted"
        );
        let server = std::thread::spawn(move || {
            futures::executor::block_on(async {
                let work = async {
                    let stream = listener.accept().await.unwrap();
                    let mut reader = BufReader::new(stream);
                    let mut input = Vec::new();

                    reader.read_until(b'\n', &mut input).await.unwrap();
                    let decoded: Request = serde_json::from_slice(&input).unwrap();

                    assert!(matches!(decoded.message, Message::Query(Query::Info {})));
                    let bytes = if oversized {
                        vec![b'x'; MAX_MESSAGE_BYTES + 1]
                    } else {
                        let mut bytes =
                            serde_json::to_vec(&Ok::<_, ControlError>(Reply::Acknowledged))
                                .unwrap();

                        bytes.push(b'\n');

                        bytes
                    };

                    let _ = reader.get_mut().write_all(&bytes).await;
                    // Keep the instance live until the client closes or the bounded timer wins.
                    let mut byte = [0];
                    let _ = reader.read(&mut byte).await;
                };

                assert!(
                    matches!(
                        select(
                            Box::pin(work),
                            Box::pin(async_io::Timer::after(Duration::from_secs(5)))
                        )
                        .await,
                        Either::Left(_)
                    ),
                    "pipe server timed out"
                );
            })
        });

        let result = local::exchange(&path, &request(), Duration::from_secs(3));

        if oversized {
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
        } else {
            assert!(matches!(result.unwrap().unwrap(), Reply::Acknowledged));
        }

        server.join().unwrap();
    }
}

#[test]
fn missing_pipe_and_stalled_read_and_write_share_one_deadline() {
    let missing = pipe::path(&identity());
    let start = Instant::now();

    assert_eq!(
        local::exchange(&missing, &request(), Duration::from_millis(40))
            .unwrap_err()
            .kind(),
        io::ErrorKind::TimedOut
    );
    assert!(start.elapsed() < Duration::from_secs(2));

    for write in [false, true] {
        let path = pipe::path(&identity());
        let listener = Listener::bind(&path).unwrap();
        let start = Instant::now();
        let mut request = request();

        if write {
            request.key = Some("x".repeat(MAX_MESSAGE_BYTES - 1024));
        }

        assert_eq!(
            local::exchange(&path, &request, Duration::from_millis(50))
                .unwrap_err()
                .kind(),
            io::ErrorKind::TimedOut
        );
        assert!(start.elapsed() < Duration::from_secs(2));
        drop(listener);
    }
}

#[test]
fn remote_and_arbitrary_device_names_are_rejected() {
    for path in [
        r"\\server\pipe\rake-test",
        r"\\.\pipe\other",
        r"\\.\pipe\rake-a\b",
        "C:\\file",
    ] {
        let result = local::exchange(
            std::path::Path::new(path),
            &request(),
            Duration::from_millis(50),
        );

        assert_eq!(
            result.unwrap_err().kind(),
            io::ErrorKind::InvalidInput,
            "{path}"
        );
    }
}

#[test]
fn discovery_skips_invalid_and_oversized_advertisements() {
    let root = std::env::temp_dir().join(format!("rake-discover-{}", identity()));

    local::prepare_directory(&root).unwrap();
    let value = serde_json::json!({
        "application_id":"windows-test", "pid":std::process::id(),
        "endpoint":{"type":"local", "path":pipe::path(&identity())},
        "info":{
            "protocol":1,"runtime_id":"00000000-0000-4000-8000-000000000001",
            "platform":"native","commands":[],"queries":["info"],
            "supported_targets":[],"completed_operations":8,"pending_operations":8,
            "idempotency_capacity":8
        }
    });

    serde_json::to_writer(
        rake_client::private::create(&root.join("good.json")).unwrap(),
        &value,
    )
    .unwrap();
    std::io::Write::write_all(
        &mut rake_client::private::create(&root.join("invalid.json")).unwrap(),
        b"invalid",
    )
    .unwrap();
    std::io::Write::write_all(
        &mut rake_client::private::create(&root.join("large.json")).unwrap(),
        &vec![b'x'; MAX_MESSAGE_BYTES + 1],
    )
    .unwrap();
    let found = local::discover(&root).unwrap();

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].application_id, "windows-test");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn pipe_moves_large_payloads_both_directions_without_loss() {
    let path = pipe::path(&identity());
    let listener = Listener::bind(&path).unwrap();

    let payload: Vec<u8> = (0..MAX_MESSAGE_BYTES + 1)
        .map(|index| (index % 251) as u8)
        .collect();

    futures::executor::block_on(async {
        let server = async {
            let mut stream = listener.accept().await.unwrap();
            let mut received = vec![0; payload.len()];

            stream.read_exact(&mut received).await.unwrap();
            assert_eq!(received, payload);
            stream.write_all(&received).await.unwrap();
            let mut end = [0];
            let _ = stream.read(&mut end).await;
        };

        let client = async {
            let mut stream = Stream::connect(&path).await.unwrap();
            let mut received = vec![0; payload.len()];

            stream.write_all(&payload).await.unwrap();
            stream.read_exact(&mut received).await.unwrap();
            assert_eq!(received, payload);
        };

        assert!(
            matches!(
                select(
                    Box::pin(futures::future::join(server, client)),
                    Box::pin(async_io::Timer::after(Duration::from_secs(3)))
                )
                .await,
                Either::Left(_)
            ),
            "large bidirectional pipe transfer stalled"
        );
    });
}
