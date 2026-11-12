//! The supervisor's bounded JSON exchange, retaining mutation keys across a lost response.

use super::super::Result;
use serde_json::{Value, json};

use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::io::{Read, Write};

pub(super) const MAX_BYTES: usize = 1024 * 1024;

pub(super) fn exchange(
    instance: &Value,
    kind: &str,
    value: Value,
    timeout: Duration,
    key: Option<&str>,
    cancelled: &AtomicBool,
) -> Result<Value> {
    let request = json!({"protocol":1, "runtime_id":instance["info"]["runtime_id"],
        "key":if kind == "command" { Some(key.map(str::to_owned).unwrap_or_else(|| uuid::Uuid::new_v4().to_string())) } else { None },
        "message":{"type":kind, "value":value}});

    let mut encoded = serde_json::to_vec(&request)?;

    encoded.push(b'\n');

    if encoded.len() > MAX_BYTES {
        return Err("request exceeds the control frame limit".into());
    }

    let path = instance["endpoint"]["path"]
        .as_str()
        .ok_or("runtime endpoint path missing")?;

    let mut response = Vec::new();

    for attempt in 0..2 {
        if cancelled.load(Ordering::Acquire) {
            return Err("build interrupted".into());
        }

        match send(path, &encoded, timeout, cancelled) {
            Ok(bytes) => {
                response = bytes;
                break;
            }
            Err(error) if attempt == 1 => return Err(error.into()),
            Err(_) => {}
        }
    }

    if response.len() > MAX_BYTES {
        return Err("runtime response exceeds the control frame limit".into());
    }

    let result: Value = serde_json::from_slice(&response)?;

    if let Some(error) = result.get("Err") {
        return Err(error.to_string().into());
    }

    result
        .get("Ok")
        .and_then(|reply| reply.get("value"))
        .cloned()
        .ok_or_else(|| "runtime reply value missing".into())
}

#[cfg(unix)]
fn send(
    path: &str,
    encoded: &[u8],
    timeout: Duration,
    cancelled: &AtomicBool,
) -> std::io::Result<Vec<u8>> {
    let mut connection = std::os::unix::net::UnixStream::connect(path)?;

    connection.set_write_timeout(Some(timeout))?;
    connection.write_all(encoded)?;
    connection.set_read_timeout(Some(Duration::from_millis(25)))?;
    let mut deadline = Instant::now() + timeout;
    let mut response = Vec::new();
    let mut buffer = [0; 65536];

    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(std::io::Error::other("build interrupted"));
        }

        if Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "runtime exchange timed out",
            ));
        }

        let remaining = (MAX_BYTES + 1 - response.len()).min(buffer.len());

        match connection.read(&mut buffer[..remaining]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "runtime disconnected during reload",
                ));
            }
            Ok(count) => {
                deadline = Instant::now() + timeout;
                response.extend_from_slice(&buffer[..count]);

                if response.len() > MAX_BYTES || response.contains(&b'\n') {
                    return Ok(response);
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error),
        }
    }
}

#[cfg(windows)]
fn send(
    path: &str,
    encoded: &[u8],
    timeout: Duration,
    cancelled: &AtomicBool,
) -> std::io::Result<Vec<u8>> {
    use futures::{
        future::{Either, select},
        io::{AsyncReadExt, AsyncWriteExt},
    };
    let deadline = Instant::now()
        .checked_add(timeout)
        .filter(|_| !timeout.is_zero())
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid runtime timeout")
        })?;

    futures::executor::block_on(async {
        let work = async {
            let mut stream = rake_client::pipe::Stream::connect(std::path::Path::new(path)).await?;

            stream.write_all(encoded).await?;
            let mut bytes = Vec::new();
            let mut buffer = [0; 65536];

            loop {
                let remaining = (MAX_BYTES + 1 - bytes.len()).min(buffer.len());
                let count = stream.read(&mut buffer[..remaining]).await?;

                if count == 0 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "runtime disconnected during reload",
                    ));
                }

                bytes.extend_from_slice(&buffer[..count]);

                if bytes.len() > MAX_BYTES || bytes.contains(&b'\n') {
                    return Ok(bytes);
                }
            }
        };

        let stop = async {
            loop {
                if cancelled.load(Ordering::Acquire) {
                    return std::io::Error::other("build interrupted");
                }

                if Instant::now() >= deadline {
                    return std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "runtime exchange timed out",
                    );
                }

                async_io::Timer::after(
                    Duration::from_millis(5)
                        .min(deadline.saturating_duration_since(Instant::now())),
                )
                .await;
            }
        };

        match select(Box::pin(stop), Box::pin(work)).await {
            Either::Left((error, _)) => Err(error),
            Either::Right((result, _)) => result,
        }
    })
}
