//! One WebSocket owner and a bounded, ordered session worker. Close never waits for queued work.

use super::wire::{Error, Request, Response};
use super::{Result, TIMEOUT};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};
use tungstenite::{Message, WebSocket};

pub(super) struct Work {
    pub request: Request,
    pub deadline: Instant,
}

struct Output {
    message: Message,
    completed: bool,
}

pub(super) struct Worker {
    input: mpsc::Receiver<Work>,
    output: mpsc::SyncSender<Output>,
    closed: Arc<AtomicBool>,
}

impl Worker {
    pub fn read(&self) -> std::result::Result<Work, mpsc::RecvTimeoutError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(mpsc::RecvTimeoutError::Disconnected);
        }
        self.input.recv_timeout(Duration::from_millis(20))
    }

    pub fn send(&self, message: Message) -> Result<()> {
        self.output
            .send(Output {
                message,
                completed: false,
            })
            .map_err(|_| "CDP reader disconnected".into())
    }

    pub fn reply(&self, response: Response) -> Result<()> {
        self.output
            .send(Output {
                message: Message::text(response.encode()?),
                completed: true,
            })
            .map_err(|_| "CDP reader disconnected".into())
    }
}

pub(super) fn serve(
    mut socket: WebSocket<TcpStream>,
    work: impl FnOnce(Worker) -> Result<()> + Send + 'static,
) -> Result<()> {
    socket
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(20)))?;
    socket
        .get_mut()
        .set_write_timeout(Some(Duration::from_secs(1)))?;
    let (input, requests) = mpsc::sync_channel(4);
    // At most two encoded messages in addition to the session's byte-bounded queue.
    let (events, output) = mpsc::sync_channel(2);
    let closed = Arc::new(AtomicBool::new(false));
    let worker = Worker {
        input: requests,
        output: events,
        closed: Arc::clone(&closed),
    };
    let thread = std::thread::spawn(move || work(worker).map_err(|error| error.to_string()));
    let mut peer_closed = false;
    let result = (|| -> Result<()> {
        let mut pending = 0usize;
        let mut busy = false;
        loop {
            for _ in 0..32 {
                match output.try_recv() {
                    Ok(output) => {
                        if output.completed {
                            pending = pending.saturating_sub(1);
                        }
                        socket.write(output.message)?;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
                }
            }
            socket.flush()?;
            if busy != (pending > 0) {
                busy = pending > 0;
                socket
                    .get_mut()
                    .set_read_timeout(Some(Duration::from_millis(if busy { 2 } else { 20 })))?;
            }
            match socket.read() {
                Ok(Message::Text(text)) => {
                    let request = match serde_json::from_str::<Request>(&text) {
                        Ok(request) => request,
                        Err(error) => {
                            socket.send(Message::text(
                                Response::new(None, Err(Error::new(-32600, error.to_string())))
                                    .encode()?,
                            ))?;
                            continue;
                        }
                    };
                    // These startup acknowledgements have no session state or runtime work.
                    if matches!(
                        request.method.as_str(),
                        "Inspector.enable" | "Runtime.runIfWaitingForDebugger"
                    ) && (request.params.is_object() || request.params.is_null())
                    {
                        socket.send(Message::text(
                            Response::new(Some(request.id), Ok(serde_json::json!({}))).encode()?,
                        ))?;
                        continue;
                    }
                    let id = request.id;
                    match input.try_send(Work {
                        request,
                        deadline: Instant::now() + TIMEOUT,
                    }) {
                        Ok(()) => pending += 1,
                        Err(mpsc::TrySendError::Full(_)) => {
                            socket.send(Message::text(
                                Response::new(
                                    Some(id),
                                    Err(Error::new(
                                        -32000,
                                        "CDP request queue is full; no operation was started",
                                    )),
                                )
                                .encode()?,
                            ))?;
                        }
                        Err(mpsc::TrySendError::Disconnected(_)) => return Ok(()),
                    }
                }
                Ok(Message::Close(_)) | Err(tungstenite::Error::ConnectionClosed) => {
                    peer_closed = true;
                    closed.store(true, Ordering::Release);
                    let _ = socket.flush();
                    return Ok(());
                }
                Ok(_) => {
                    socket.flush()?;
                }
                Err(tungstenite::Error::Io(error))
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(error) => return Err(error.into()),
            }
        }
    })();
    closed.store(true, Ordering::Release);
    drop(input);
    drop(output);
    let _ = socket.close(None);
    let worker_result = thread.join().map_err(|_| "CDP session worker panicked")?;
    result?;
    if peer_closed {
        return Ok(());
    }
    worker_result.map_err(Into::into)
}

#[cfg(test)]
mod tests {
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
}
