//! Per-user instance ownership and bounded activation forwarding.

use futures::io::{AsyncReadExt, AsyncWriteExt};
use std::{fs::File, io, time::Duration};

#[cfg(unix)]
type Stream = async_io::Async<std::os::unix::net::UnixStream>;
#[cfg(windows)]
type Stream = crate::pipe::Stream;
#[cfg(unix)]
type Listener = async_io::Async<std::os::unix::net::UnixListener>;
#[cfg(windows)]
type Listener = crate::pipe::Listener;

/// Holds the kernel lock for the lifetime of the primary process.
/// Lock files deliberately persist: deleting them would permit split ownership.
pub struct Primary {
    _lock: File,
    listener: Listener,
}

/// Claims instance ownership, or forwards arguments and returns `None`.
///
/// # Errors
/// Returns private-directory, lock, transport or acknowledgement failures.
/// A failed forward never starts a second instance.
pub fn launch(id: &str, arguments: &[String]) -> io::Result<Option<Primary>> {
    let root = crate::local::directory();

    crate::private::prepare_directory(&root)?;
    #[cfg(unix)]
    let root_bytes = {
        use std::os::unix::ffi::OsStrExt;

        root.as_os_str().as_bytes().to_vec()
    };
    #[cfg(windows)]
    let root_bytes = {
        use std::os::windows::ffi::OsStrExt;

        root.as_os_str()
            .encode_wide()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>()
    };
    let key = instance_key(id, &root_bytes);
    let lock_path = root.join(format!("instance-{key}.lock"));

    let lock = match crate::private::create(&lock_path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            crate::private::open(&lock_path)?
        }
        Err(error) => return Err(error),
    };

    #[cfg(unix)]
    let path = root.join(format!("instance-{key}.sock"));

    #[cfg(windows)]
    let path = crate::pipe::path(&format!("instance-{key}"));

    match lock.try_lock() {
        Ok(()) => {
            #[cfg(unix)]
            let listener = {
                match std::fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }

                async_io::Async::new(std::os::unix::net::UnixListener::bind(&path)?)?
            };

            #[cfg(windows)]
            let listener = Listener::bind(&path)?;

            Ok(Some(Primary {
                _lock: lock,
                listener,
            }))
        }
        Err(std::fs::TryLockError::WouldBlock) => {
            let payload = serde_json::to_vec(arguments)?;

            if payload.len() > 65536 {
                return Err(io::ErrorKind::InvalidInput.into());
            }

            futures::executor::block_on(bounded(async {
                // The winner can hold the lock before its listener is ready.
                let mut stream = loop {
                    match connect(&path).await {
                        Ok(stream) => break stream,
                        Err(error)
                            if matches!(
                                error.kind(),
                                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                            ) =>
                        {
                            async_io::Timer::after(Duration::from_millis(10)).await;
                        }
                        Err(error) => return Err(error),
                    }
                };

                stream
                    .write_all(&(payload.len() as u32).to_le_bytes())
                    .await?;
                stream.write_all(&payload).await?;
                let mut ack = [0];

                stream.read_exact(&mut ack).await?;

                if ack != [1] {
                    return Err(io::ErrorKind::InvalidData.into());
                }

                Ok(())
            }))?;

            Ok(None)
        }
        Err(std::fs::TryLockError::Error(error)) => Err(error),
    }
}

async fn connect(path: &std::path::Path) -> io::Result<Stream> {
    #[cfg(unix)]
    {
        Stream::connect(path).await
    }

    #[cfg(windows)]
    {
        Stream::connect(path).await
    }
}

async fn bounded<T>(operation: impl std::future::Future<Output = io::Result<T>>) -> io::Result<T> {
    use futures::{FutureExt, future::Either};

    match futures::future::select(
        operation.boxed_local(),
        async_io::Timer::after(Duration::from_secs(5)),
    )
    .await
    {
        Either::Left((result, _)) => result,
        Either::Right(_) => Err(io::ErrorKind::TimedOut.into()),
    }
}

impl Primary {
    /// Receives one bounded argument batch and acknowledges delivery.
    ///
    /// # Errors
    /// Returns malformed-frame, timeout or transport failures.
    pub async fn receive(&self) -> io::Result<Vec<String>> {
        #[cfg(unix)]
        let (mut stream, _) = self.listener.accept().await?;

        #[cfg(windows)]
        let mut stream = self.listener.accept().await?;

        bounded(async {
            let mut length = [0; 4];

            stream.read_exact(&mut length).await?;
            let length = u32::from_le_bytes(length) as usize;

            if length > 65536 {
                return Err(io::ErrorKind::InvalidData.into());
            }

            let mut bytes = vec![0; length];

            stream.read_exact(&mut bytes).await?;
            let arguments = serde_json::from_slice(&bytes)?;

            stream.write_all(&[1]).await?;

            Ok(arguments)
        })
        .await
    }
}

fn instance_key(id: &str, root: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();

    hash.update((id.len() as u64).to_le_bytes());
    hash.update(id.as_bytes());
    hash.update(root);

    // 128 bits keep Unix socket paths short while retaining collision resistance.
    format!("{:x}", hash.finalize())[..32].to_owned()
}

#[cfg(test)]
mod tests {
    #[test]
    fn instance_key_is_stable() {
        assert_eq!(
            super::instance_key("com.example.app", b"/tmp/rake-1000"),
            "06fecfe32ee0b7f2495cc581528b971b"
        );
    }
}
