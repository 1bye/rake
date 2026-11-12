//! Unix-domain development transport. Private discovery directories and sockets
//! restrict access to the current user. Owned protocol frames are decoded/encoded
//! on the blocking pool; host dispatch and all UI state remain on the local thread.

use std::fs;
#[cfg(unix)]
use std::fs::OpenOptions;
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(unix)]
use async_io::Async;
#[cfg(unix)]
type Listener = Async<UnixListener>;
#[cfg(unix)]
type Stream = Async<UnixStream>;
use futures::future::{Either, select};
use futures::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use futures::stream::FuturesUnordered;
use futures::{FutureExt, StreamExt};
#[cfg(windows)]
use rake_client::pipe::{Listener, Stream};

use super::{HostClient, MAX_MESSAGE_BYTES, RuntimeClient, RuntimeInfo};
use crate::router::Route;

pub use rake_client::local::{
    Endpoint, Instance, directory, discover, exchange, prepare_directory,
};

/// Owns a listener and its discovery files. Dropping it removes both files.
pub struct LocalServer {
    listener: Listener,
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
        #[cfg(unix)]
        let socket = path.join(format!("{stem}.sock"));
        #[cfg(windows)]
        let socket = rake_client::pipe::path(&stem);
        let manifest = path.join(format!("{stem}.json"));
        #[cfg(unix)]
        let listener = Async::<UnixListener>::bind(&socket)?;
        #[cfg(windows)]
        let listener = Listener::bind(&socket)?;

        let server = Self {
            listener,
            socket,
            manifest,
        };

        #[cfg(unix)]
        fs::set_permissions(&server.socket, fs::Permissions::from_mode(0o600))?;
        let instance = Instance {
            application_id: application_id.into(),
            pid: std::process::id(),
            info,
            endpoint: Endpoint::Local {
                path: server.socket.clone(),
            },
        };

        #[cfg(unix)]
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&server.manifest)?;

        #[cfg(windows)]
        let mut file = rake_client::private::create(&server.manifest)?;

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
                Some(self.accept().await?)
            } else {
                match select(Box::pin(self.accept()), Box::pin(connections.next())).await {
                    Either::Left((accepted, _)) => Some(accepted?),
                    Either::Right(_) => None,
                }
            };

            if let Some(stream) = stream {
                connections.push(serve_one(stream, client.clone()).boxed_local());
            }
        }
    }

    #[cfg(unix)]
    async fn accept(&self) -> io::Result<Stream> {
        loop {
            match self.listener.accept().await {
                Ok((stream, _)) => return Ok(stream),
                Err(error) if recoverable_accept_error(&error) => {
                    tracing::warn!(%error, "local control accept failed; retrying");
                    crate::http::sleep(Duration::from_millis(100)).await;
                }
                Err(error) => return Err(error),
            }
        }
    }
    #[cfg(windows)]
    async fn accept(&self) -> io::Result<Stream> {
        self.listener.accept().await
    }
}

#[cfg(unix)]
fn recoverable_accept_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::Interrupted
            | std::io::ErrorKind::WouldBlock
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
    ) || matches!(
        error.raw_os_error(),
        Some(libc::EMFILE | libc::ENFILE | libc::ENOBUFS | libc::ENOMEM)
    )
}

// Framed input is bounded before allocating or decoding it.
async fn serve_one(stream: Stream, client: HostClient) {
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
        let request = blocking::unblock(move || serde_json::from_slice(&bytes)).await?;
        let reply = client.request_async(request).await;
        let mut bytes = blocking::unblock(move || encode_reply(&reply)).await?;

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

fn encode_reply(value: &impl serde::Serialize) -> io::Result<Vec<u8>> {
    struct Bounded(Vec<u8>);

    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.0.len().saturating_add(bytes.len()) >= MAX_MESSAGE_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "control response too large",
                ));
            }

            self.0.extend_from_slice(bytes);

            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let mut writer = Bounded(Vec::new());

    serde_json::to_writer(&mut writer, value)?;

    Ok(writer.0)
}

impl Drop for LocalServer {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.manifest);
        #[cfg(unix)]
        let _ = fs::remove_file(&self.socket);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn temporary_accept_errors_recover_but_bad_listeners_fail() {
        for code in [
            libc::EMFILE,
            libc::ENFILE,
            libc::ECONNABORTED,
            libc::EINTR,
            libc::ENOBUFS,
            libc::ENOMEM,
        ] {
            assert!(
                recoverable_accept_error(&std::io::Error::from_raw_os_error(code)),
                "errno {code}"
            );
        }

        for code in [libc::EBADF, libc::EINVAL, libc::ENOTSOCK] {
            assert!(
                !recoverable_accept_error(&std::io::Error::from_raw_os_error(code)),
                "errno {code}"
            );
        }
    }

    #[test]
    fn owned_reply_encoding_preserves_json_and_stops_at_the_transport_limit() {
        let value = serde_json::json!({"text":"\"\\\né🦀", "values":[1,true,null]});
        let encoded = encode_reply(&value).unwrap();

        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&encoded).unwrap(),
            value
        );
        assert!(encode_reply(&"x".repeat(MAX_MESSAGE_BYTES - 3)).is_ok());
        assert!(encode_reply(&"x".repeat(MAX_MESSAGE_BYTES - 2)).is_err());
        assert!(encode_reply(&"\0".repeat(MAX_MESSAGE_BYTES / 2)).is_err());
    }
}
