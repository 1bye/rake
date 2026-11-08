//! Owned Swift capture process and bounded keyframe-aware fanout.
use super::{Error, Result};
use crate::project_build::processes::ProcessGroup;
use serde_json::Value;

use std::{
    collections::VecDeque,
    io::{Read, Write},
    os::unix::process::CommandExt,
    path::Path,
    process::{ChildStdin, Command, Stdio},
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone)]
struct Packet {
    sequence: u64,
    created: Instant,
    metadata: Value,
    bytes: Arc<Vec<u8>>,
}

#[derive(Default)]
struct Buffer {
    packets: VecDeque<Packet>,
    bytes: usize,
    sequence: u64,
    config: Option<Arc<Vec<u8>>>,
    error: Option<String>,
    closed: bool,
    viewers: usize,
    last_key: Option<Instant>,
}

struct Shared {
    buffer: Mutex<Buffer>,
    changed: Condvar,
    process: Mutex<Option<ProcessGroup>>,
    input: Mutex<Option<ChildStdin>>,
}

impl Shared {
    fn stop_process(&self) {
        let process = self
            .process
            .lock()
            .expect("capture process lock poisoned")
            .take();

        if let Some(mut process) = process {
            let pid = process.0.id() as i32;

            // SAFETY: the helper owns the process group created at spawn.
            unsafe {
                libc::kill(-pid, libc::SIGTERM);
            }
            let deadline = Instant::now() + Duration::from_secs(2);

            while process.0.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }

            // SAFETY: terminate only this helper's group, including inherited pipes.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
            let _ = process.0.wait();
        }
    }

    fn keyframe(&self) {
        let mut buffer = self.buffer.lock().expect("capture buffer lock poisoned");

        if buffer.closed
            || buffer
                .last_key
                .is_some_and(|last| last.elapsed() < Duration::from_millis(250))
        {
            return;
        }

        buffer.last_key = Some(Instant::now());
        drop(buffer);

        if let Some(input) = self
            .input
            .lock()
            .expect("capture input lock poisoned")
            .as_mut()
        {
            let _ = input.write_all(b"K");
            let _ = input.flush();
        }
    }
}

pub(super) struct Capture {
    shared: Arc<Shared>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl Capture {
    pub(super) fn start(
        helper: &Path,
        pid: u64,
        window: &Value,
        codec: &str,
        epoch: &str,
    ) -> Result<Self> {
        let arg = |value: &Value| {
            value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string())
        };

        let mut command = Command::new(helper);

        command.args([
            pid.to_string(),
            arg(&window["native_id"]),
            arg(&window["frame"]["width"]),
            arg(&window["frame"]["height"]),
            codec.into(),
        ]);

        Self::spawn(command, epoch)
    }

    fn spawn(mut command: Command, epoch: &str) -> Result<Self> {
        let child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()?;

        let mut process = ProcessGroup(child);
        let mut output = process.0.stdout.take().expect("capture stdout piped");
        let mut errors = process.0.stderr.take().expect("capture stderr piped");
        let input = process.0.stdin.take();

        let shared = Arc::new(Shared {
            buffer: Mutex::default(),
            changed: Condvar::new(),
            process: Mutex::new(Some(process)),
            input: Mutex::new(input),
        });

        let receiver = shared.clone();
        let epoch = epoch.to_owned();

        let worker = thread::spawn(move || {
            // Drain stderr continuously, retaining only the original 4096-byte diagnostic.
            let stderr = thread::spawn(move || {
                let mut retained = Vec::new();
                let mut chunk = [0; 4096];

                loop {
                    match errors.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(count) => {
                            let keep = count.min(4096 - retained.len());

                            retained.extend_from_slice(&chunk[..keep]);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(_) => break,
                    }
                }

                retained
            });

            let result = (|| -> Result<()> {
                loop {
                    let (metadata, packet) = read_packet(&mut output, &epoch)?;

                    let mut buffer = receiver
                        .buffer
                        .lock()
                        .expect("capture buffer lock poisoned");

                    if buffer.closed {
                        return Ok(());
                    }

                    buffer.sequence += 1;
                    let sequence = buffer.sequence;
                    let packet = Arc::new(packet);

                    if metadata["kind"] == "config" {
                        buffer.config = Some(packet.clone());
                    }

                    buffer.bytes += packet.len();
                    buffer.packets.push_back(Packet {
                        sequence,
                        created: Instant::now(),
                        metadata,
                        bytes: packet,
                    });

                    while buffer.packets.len() > 120 || buffer.bytes > 8 * 1024 * 1024 {
                        buffer.bytes -= buffer
                            .packets
                            .pop_front()
                            .expect("nonempty bounded capture queue")
                            .bytes
                            .len();
                    }

                    receiver.changed.notify_all();
                }
            })();

            receiver.stop_process();
            let detail = String::from_utf8_lossy(&stderr.join().unwrap_or_default())
                .trim()
                .to_owned();

            let mut buffer = receiver
                .buffer
                .lock()
                .expect("capture buffer lock poisoned");

            buffer.error = Some(if !detail.is_empty() {
                detail
            } else {
                result
                    .err()
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "Native capture ended".into())
            });
            receiver.changed.notify_all();
        });

        Ok(Self {
            shared,
            worker: Mutex::new(Some(worker)),
        })
    }

    pub(super) fn error(&self) -> Option<String> {
        self.shared
            .buffer
            .lock()
            .expect("capture buffer lock poisoned")
            .error
            .clone()
    }

    pub(super) fn viewers(&self) -> usize {
        self.shared
            .buffer
            .lock()
            .expect("capture buffer lock poisoned")
            .viewers
    }

    pub(super) fn viewer(self: &Arc<Self>) -> Viewer {
        self.shared
            .buffer
            .lock()
            .expect("capture buffer lock poisoned")
            .viewers += 1;

        Viewer {
            capture: self.clone(),
        }
    }

    pub(super) fn close(&self) {
        {
            let mut buffer = self
                .shared
                .buffer
                .lock()
                .expect("capture buffer lock poisoned");

            buffer.closed = true;
            self.shared.changed.notify_all();
        }
        self.shared.stop_process();
        self.shared
            .input
            .lock()
            .expect("capture input lock poisoned")
            .take();

        if let Some(worker) = self
            .worker
            .lock()
            .expect("capture worker lock poisoned")
            .take()
        {
            let _ = worker.join();
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.close();
    }
}

pub(super) struct Viewer {
    capture: Arc<Capture>,
}

impl Viewer {
    pub(super) fn stream(&self, writer: &mut impl Write) -> std::io::Result<()> {
        let shared = &self.capture.shared;

        let mut cursor = shared
            .buffer
            .lock()
            .expect("capture buffer lock poisoned")
            .sequence;

        shared.keyframe();
        let mut waiting = true;

        loop {
            let (pending, config) = {
                let buffer = shared.buffer.lock().expect("capture buffer lock poisoned");

                let (buffer, _) = shared
                    .changed
                    .wait_timeout_while(buffer, Duration::from_secs(1), |buffer| {
                        !buffer.closed && buffer.error.is_none() && buffer.sequence <= cursor
                    })
                    .expect("capture buffer lock poisoned");

                if buffer.closed || buffer.error.is_some() {
                    return Ok(());
                }

                (
                    buffer
                        .packets
                        .iter()
                        .filter(|packet| packet.sequence > cursor)
                        .cloned()
                        .collect::<Vec<_>>(),
                    buffer.config.clone(),
                )
            };

            if pending.is_empty() {
                writer.write_all(&packet(&serde_json::json!({"kind":"ping"}), &[0]))?;
                continue;
            }

            if pending[0].sequence != cursor + 1
                || pending[0].created.elapsed() > Duration::from_millis(150)
            {
                waiting = true;
            }

            cursor = pending.last().expect("nonempty capture batch").sequence;
            let start = if waiting {
                let Some(index) = pending.iter().rposition(|packet| {
                    packet.metadata["key"] == true || packet.metadata["kind"] == "jpeg"
                }) else {
                    shared.keyframe();
                    continue;
                };

                if let Some(config) = config {
                    writer.write_all(&config)?;
                }

                waiting = false;

                index
            } else {
                0
            };

            for packet in &pending[start..] {
                writer.write_all(&packet.bytes)?;
            }
        }
    }
}

impl Drop for Viewer {
    fn drop(&mut self) {
        self.capture
            .shared
            .buffer
            .lock()
            .expect("capture buffer lock poisoned")
            .viewers -= 1;
    }
}

fn exact(reader: &mut impl Read, bytes: &mut [u8]) -> Result<()> {
    reader.read_exact(bytes).map_err(|error| {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            Error::new(502, "Native capture ended")
        } else {
            error.into()
        }
    })
}

fn read_packet(reader: &mut impl Read, epoch: &str) -> Result<(Value, Vec<u8>)> {
    let mut sizes = [0; 8];

    exact(reader, &mut sizes)?;
    let header_size =
        u32::from_be_bytes(sizes[..4].try_into().expect("four header bytes")) as usize;

    let size = u32::from_be_bytes(sizes[4..].try_into().expect("four payload bytes")) as usize;

    if !(1..=4096).contains(&header_size) || !(1..=2 * 1024 * 1024).contains(&size) {
        return Err(Error::new(400, "Capture exceeded its bounded packet size"));
    }

    let mut header = vec![0; header_size];

    exact(reader, &mut header)?;
    let mut metadata: Value = serde_json::from_slice(&header)?;
    let mut payload = vec![0; size];

    exact(reader, &mut payload)?;

    if !matches!(metadata["kind"].as_str(), Some("config" | "frame" | "jpeg"))
        || ["width", "height"].iter().any(|key| {
            !metadata[*key]
                .as_u64()
                .is_some_and(|value| (1..=4096).contains(&value))
        })
    {
        return Err(Error::new(400, "Invalid capture packet"));
    }

    metadata["epoch"] = Value::from(epoch);
    let bytes = packet(&metadata, &payload);

    Ok((metadata, bytes))
}

fn packet(metadata: &Value, payload: &[u8]) -> Vec<u8> {
    let header = metadata.to_string().into_bytes();
    let mut packet = Vec::with_capacity(8 + header.len() + payload.len());

    packet.extend_from_slice(&(header.len() as u32).to_be_bytes());
    packet.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    packet.extend(header);
    packet.extend(payload);

    packet
}

#[cfg(test)]
mod tests;
