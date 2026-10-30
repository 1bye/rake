//! Unix-domain development transport. Private discovery directories and sockets
//! restrict access to the current user. No UI state crosses a thread boundary.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_io::Async;
use futures::future::{Either, select};
use futures::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use futures::stream::FuturesUnordered;
use futures::{FutureExt, StreamExt};

use super::{HostClient, MAX_MESSAGE_BYTES, RuntimeClient, RuntimeInfo};
use crate::router::Route;

pub use rake_client::local::{
    Endpoint, Instance, directory, discover, exchange, prepare_directory,
};

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
        Self::bind_host(&HostClient::new(client.clone()), application_id, path)
    }

    pub(crate) fn bind_host(
        client: &HostClient,
        application_id: &str,
        path: &Path,
    ) -> io::Result<Self> {
        Self::bind_info(client.info(), application_id, path)
    }

    fn bind_info(info: RuntimeInfo, application_id: &str, path: &Path) -> io::Result<Self> {
        prepare_directory(path)?;
        let stem = info.runtime_id.to_string();
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
            info,
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
        self.run_host(HostClient::new(client)).await
    }

    pub(crate) async fn run_host(&self, client: HostClient) -> io::Result<()> {
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
async fn serve_one(stream: Async<UnixStream>, client: HostClient) {
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

        let _response = client.begin_response();
        let reply = client.request_async(serde_json::from_slice(&bytes)?).await;
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
