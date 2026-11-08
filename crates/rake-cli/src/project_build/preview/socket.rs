//! Bounded RFC 6455 text controls; no fragmentation, binary input or retrying mutations.
use super::{Error, Preview, Result};
use crate::project_build::serve::Request;
use base64::Engine;
use serde_json::{Value, json};

use std::{
    io::{Read, Write},
    net::{Shutdown, TcpStream},
    sync::{
        Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

struct Channel {
    writer: Mutex<TcpStream>,
    connection: TcpStream,
    closed: AtomicBool,
    wake: Condvar,
    waiting: Mutex<()>,
}

impl Channel {
    fn send(&self, value: &Value) -> std::io::Result<()> {
        write_frame(
            &mut *self
                .writer
                .lock()
                .expect("preview socket write lock poisoned"),
            1,
            &serde_json::to_vec(value)?,
        )
    }

    fn frame(&self, opcode: u8, payload: &[u8]) -> std::io::Result<()> {
        write_frame(
            &mut *self
                .writer
                .lock()
                .expect("preview socket write lock poisoned"),
            opcode,
            payload,
        )
    }

    fn close(&self) {
        self.closed.store(true, Ordering::Release);
        let _ = self.connection.shutdown(Shutdown::Both);

        self.wake.notify_all();
    }
}

pub(super) fn control(
    preview: &Preview,
    stream: &mut TcpStream,
    request: &Request,
    token: &str,
) -> Result<()> {
    let accept = upgrade(request, token)?;

    write!(
        stream,
        "HTTP/1.0 101 Switching Protocols\r\nServer: rake\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Protocol: rake-preview-v1\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    )?;
    let channel = Channel {
        writer: Mutex::new(stream.try_clone()?),
        connection: stream.try_clone()?,
        closed: AtomicBool::new(false),
        wake: Condvar::new(),
        waiting: Mutex::new(()),
    };

    std::thread::scope(|scope| {
        scope.spawn(|| {
            while !channel.closed.load(Ordering::Acquire) {
                let state = preview
                    .state()
                    .unwrap_or_else(|error| json!({"error":error.to_string()}));

                if channel
                    .send(&json!({"type":"state","value":state}))
                    .is_err()
                {
                    channel.close();
                    break;
                }

                let guard = channel
                    .waiting
                    .lock()
                    .expect("preview socket wait lock poisoned");

                let _ = channel
                    .wake
                    .wait_timeout_while(guard, Duration::from_millis(250), |_| {
                        !channel.closed.load(Ordering::Acquire)
                    });
            }
        });
        let result = (|| -> std::io::Result<()> {
            loop {
                let (opcode, payload) = read_frame(stream)?;

                match opcode {
                    8 => {
                        channel.frame(8, &[])?;
                        break;
                    }
                    9 => {
                        channel.frame(10, &payload)?;
                        continue;
                    }
                    10 => continue,
                    _ => {}
                }

                let body: Value = serde_json::from_slice(&payload)?;

                if !body.is_object() {
                    return Err(invalid("Expected preview control object"));
                }

                let id = body["id"]
                    .as_u64()
                    .filter(|id| *id < 1 << 53)
                    .ok_or_else(|| invalid("Expected control request ID"))?;

                let default = json!({});

                let result = preview.handle(
                    body["action"].as_str().unwrap_or_default(),
                    body.get("body").unwrap_or(&default),
                );

                let response = match result {
                    Ok(value) => json!({"type":"reply","id":id,"value":value}),
                    Err(error) => json!({"type":"reply","id":id,"error":error.to_string()}),
                };

                channel.send(&response)?;
            }

            Ok(())
        })();

        let _ = result;

        channel.close();
    });

    Ok(())
}

fn upgrade(request: &Request, token: &str) -> Result<String> {
    use subtle::ConstantTimeEq;
    let protocols = request
        .header("Sec-WebSocket-Protocol")
        .split(',')
        .map(str::trim)
        .collect::<Vec<_>>();

    let expected = format!("rake-token.{token}");

    if request.header("Origin") != format!("http://{}", request.header("Host"))
        || !protocols.contains(&"rake-preview-v1")
        || !protocols
            .iter()
            .any(|protocol| bool::from(protocol.as_bytes().ct_eq(expected.as_bytes())))
    {
        return Err(Error::new(403, "Preview socket authentication failed"));
    }

    let key = request.header("Sec-WebSocket-Key");

    if !base64::engine::general_purpose::STANDARD
        .decode(key)
        .is_ok_and(|key| key.len() == 16)
        || request.header("Sec-WebSocket-Version") != "13"
        || !request.header("Upgrade").eq_ignore_ascii_case("websocket")
        || !request
            .header("Connection")
            .to_ascii_lowercase()
            .contains("upgrade")
    {
        return Err(Error::new(400, "Invalid WebSocket upgrade"));
    }

    Ok(tungstenite::handshake::derive_accept_key(key.as_bytes()))
}

fn invalid(message: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message)
}

fn read_frame(reader: &mut impl Read) -> std::io::Result<(u8, Vec<u8>)> {
    let mut header = [0; 2];

    reader.read_exact(&mut header)?;
    let opcode = header[0] & 15;

    if header[0] & 0xf0 != 0x80 || header[1] & 0x80 == 0 || !matches!(opcode, 1 | 8 | 9 | 10) {
        return Err(invalid("Unsupported WebSocket frame"));
    }

    let size = match header[1] & 127 {
        126 => {
            let mut size = [0; 2];

            reader.read_exact(&mut size)?;

            u16::from_be_bytes(size) as u64
        }
        127 => {
            let mut size = [0; 8];

            reader.read_exact(&mut size)?;

            u64::from_be_bytes(size)
        }
        size => size as u64,
    };

    if size > 16384 || (opcode >= 8 && size > 125) {
        return Err(invalid("Preview control frame too large"));
    }

    let mut mask = [0; 4];

    reader.read_exact(&mut mask)?;
    let mut payload = vec![0; size as usize];

    reader.read_exact(&mut payload)?;

    for (index, byte) in payload.iter_mut().enumerate() {
        *byte ^= mask[index % 4];
    }

    Ok((opcode, payload))
}

fn write_frame(writer: &mut impl Write, opcode: u8, payload: &[u8]) -> std::io::Result<()> {
    if payload.len() > 256 * 1024 {
        return Err(invalid("Preview response too large"));
    }

    let mut header = vec![0x80 | opcode];

    match payload.len() {
        size if size < 126 => header.push(size as u8),
        size if size < 65536 => {
            header.push(126);
            header.extend_from_slice(&(size as u16).to_be_bytes());
        }
        size => {
            header.push(127);
            header.extend_from_slice(&(size as u64).to_be_bytes());
        }
    }

    writer.write_all(&header)?;

    writer.write_all(payload)
}

#[cfg(test)]
mod tests;
