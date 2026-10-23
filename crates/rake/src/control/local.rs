//! Unix-domain development transport. Private discovery directories and sockets
//! restrict access to the current user. No UI state crosses a thread boundary.

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_io::Async;
use futures::future::{Either, select};
use futures::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use futures::stream::FuturesUnordered;
use futures::{FutureExt, StreamExt};
use serde::{Deserialize, Serialize};

use super::{ControlError, MAX_MESSAGE_BYTES, Reply, Request, RuntimeClient, RuntimeInfo};
use crate::router::Route;

/// A discovered runtime endpoint. Credentials are kept only in owner-readable files.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Endpoint {
    /// Permission-restricted Unix socket.
    Local {
        /// Socket path.
        path: PathBuf,
    },
    /// Authenticated development broker, addressed by runtime ID.
    Web {
        /// Loopback HTTP broker base URL.
        url: String,
        /// Random development session credential.
        token: String,
    },
}

/// An advertisement written only while a development endpoint is owned by its host.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Instance {
    /// Application name supplied by the runner.
    pub application_id: String,
    /// Native application or broker process ID.
    pub pid: u32,
    /// Version, runtime identity and actual capabilities.
    pub info: RuntimeInfo,
    /// Local or browser-initiated broker transport.
    pub endpoint: Endpoint,
}

/// Default short socket directory, overridable by `RAKE_CONTROL_DIR`.
pub fn directory() -> PathBuf {
    std::env::var_os("RAKE_CONTROL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            // SAFETY: geteuid has no pointer or threading preconditions.
            PathBuf::from(format!("/tmp/rake-{}", unsafe { libc::geteuid() }))
        })
}

/// Creates or validates an owner-only directory; symlinks and other owners are rejected.
pub fn prepare_directory(path: &Path) -> io::Result<()> {
    match DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }

    let metadata = fs::symlink_metadata(path)?;
    // SAFETY: geteuid has no pointer or threading preconditions.
    let uid = unsafe { libc::geteuid() };

    if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "control directory must be an owned directory with mode 0700",
        ));
    }

    Ok(())
}

/// Reads bounded, owner-only instance advertisements. A crashed host can leave stale
/// files; clients verify runtime identity by querying the endpoint before using it.
pub fn discover(path: &Path) -> io::Result<Vec<Instance>> {
    prepare_directory(path)?;
    let mut instances = Vec::new();

    for entry in fs::read_dir(path)? {
        let path = entry?.path();

        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }

        let metadata = fs::symlink_metadata(&path)?;
        // SAFETY: geteuid has no pointer or threading preconditions.
        let uid = unsafe { libc::geteuid() };

        if !metadata.is_file() || metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
            continue;
        }

        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)?;

        let mut bytes = Vec::new();

        file.take(MAX_MESSAGE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;

        if bytes.len() <= MAX_MESSAGE_BYTES
            && let Ok(instance) = serde_json::from_slice(&bytes)
        {
            instances.push(instance);
        }
    }

    Ok(instances)
}

/// Owns a listener and its discovery files. Dropping it removes both files.
pub struct LocalServer {
    listener: Async<UnixListener>,
    socket: PathBuf,
    manifest: PathBuf,
}

impl LocalServer {
    /// Binds a new runtime socket. Enable only for explicitly requested development.
    pub fn bind<R: Route>(
        client: &RuntimeClient<R>,
        application_id: &str,
        path: &Path,
    ) -> io::Result<Self> {
        prepare_directory(path)?;
        let stem = client.info().runtime_id.to_string();
        let socket = path.join(format!("{stem}.sock"));
        let manifest = path.join(format!("{stem}.json"));
        let listener = Async::<UnixListener>::bind(&socket)?;

        let server = Self {
            listener,
            socket,
            manifest,
        };

        fs::set_permissions(&server.socket, fs::Permissions::from_mode(0o600))?;
        let instance = Instance {
            application_id: application_id.into(),
            pid: std::process::id(),
            info: client.info(),
            endpoint: Endpoint::Local {
                path: server.socket.clone(),
            },
        };

        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&server.manifest)?;

        serde_json::to_writer(&mut file, &instance)?;
        file.flush()?;

        Ok(server)
    }

    /// Drives a bounded set of connections on the caller's local event loop.
    /// Each connection has a five-second I/O deadline and submits one owned message.
    pub async fn run<R: Route>(&self, client: RuntimeClient<R>) -> io::Result<()> {
        let mut connections = FuturesUnordered::new();

        loop {
            if connections.len() == 32 {
                connections.next().await;

                continue;
            }

            let stream = if connections.is_empty() {
                Some(self.listener.accept().await?.0)
            } else {
                match select(
                    Box::pin(self.listener.accept()),
                    Box::pin(connections.next()),
                )
                .await
                {
                    Either::Left((accepted, _)) => Some(accepted?.0),
                    Either::Right(_) => None,
                }
            };

            if let Some(stream) = stream {
                connections.push(serve_one(stream, client.clone()).boxed_local());
            }
        }
    }
}

// Framed input is bounded before allocating or decoding it.
async fn serve_one<R: Route>(stream: Async<UnixStream>, client: RuntimeClient<R>) {
    let work = async move {
        let mut reader = BufReader::new(stream);
        let mut bytes = Vec::new();

        loop {
            let available = reader.fill_buf().await?;

            if available.is_empty() {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
            }

            let count = available
                .iter()
                .position(|byte| *byte == b'\n')
                .map(|index| index + 1)
                .unwrap_or(available.len());

            if bytes.len() + count > MAX_MESSAGE_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "control frame too large",
                ));
            }

            bytes.extend_from_slice(&available[..count]);
            reader.consume_unpin(count);

            if bytes.last() == Some(&b'\n') {
                break;
            }
        }

        let reply = client.request(serde_json::from_slice(&bytes)?);
        let mut bytes = serde_json::to_vec(&reply)?;

        if bytes.len() + 1 > MAX_MESSAGE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "control response too large",
            ));
        }

        bytes.push(b'\n');
        reader.get_mut().write_all(&bytes).await?;

        Ok::<_, io::Error>(())
    };

    let _ = select(
        Box::pin(work),
        Box::pin(crate::http::sleep(Duration::from_secs(5))),
    )
    .await;
}

impl Drop for LocalServer {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.manifest);
        let _ = fs::remove_file(&self.socket);
    }
}

/// Blocking terminal-side exchange with a finite I/O deadline. The application-side
/// server remains local/async. A lost response can be retried using the same key.
pub fn exchange(
    path: &Path,
    request: &Request,
    timeout: Duration,
) -> io::Result<Result<Reply, ControlError>> {
    let mut stream = UnixStream::connect(path)?;

    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let mut bytes = serde_json::to_vec(request)?;

    if bytes.len() + 1 > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "control request too large",
        ));
    }

    bytes.push(b'\n');
    stream.write_all(&bytes)?;
    let mut reply = Vec::new();

    std::io::BufRead::read_until(
        &mut std::io::BufReader::new(stream.take(MAX_MESSAGE_BYTES as u64 + 1)),
        b'\n',
        &mut reply,
    )?;

    if reply.len() > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "control response too large",
        ));
    }

    Ok(serde_json::from_slice(&reply)?)
}
