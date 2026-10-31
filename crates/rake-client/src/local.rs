//! Owner-only Unix discovery and bounded blocking socket exchanges.

pub use crate::{Endpoint, Instance};
use async_io::{Async, Timer};
use futures::future::{Either, select};
use futures::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use rake_protocol::control::{ControlError, MAX_MESSAGE_BYTES, Reply, Request};
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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
///
/// # Errors
/// Returns filesystem errors, or `PermissionDenied` if the existing path is
/// a symlink, is owned by another user, or grants group/other access.
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
///
/// # Errors
/// Returns directory validation and filesystem read errors. Oversized or invalid
/// advertisement contents are ignored; files with unsafe permissions are skipped.
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

/// Blocking terminal-side exchange with one total deadline for connection, writes
/// and reads. A lost response can be retried using the same key.
///
/// # Errors
/// Returns `InvalidInput` for zero/unrepresentable timeouts or oversized requests,
/// `TimedOut` when the total deadline elapses, and `InvalidData` for oversized or
/// malformed replies. Other socket I/O failures retain their original error kind.
pub fn exchange(
    path: &Path,
    request: &Request,
    timeout: Duration,
) -> io::Result<Result<Reply, ControlError>> {
    let timeout = crate::remaining(timeout)?;

    let deadline = Instant::now()
        .checked_add(timeout)
        .filter(|_| !timeout.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid control timeout"))?;

    let mut bytes = serde_json::to_vec(request)?;

    if bytes.len() >= MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "control request too large",
        ));
    }

    bytes.push(b'\n');

    futures::executor::block_on(async {
        let exchange = async {
            let mut stream = Async::<UnixStream>::connect(path).await?;

            stream.write_all(&bytes).await?;
            let mut reply = Vec::new();

            let mut reader =
                BufReader::new(AsyncReadExt::take(stream, MAX_MESSAGE_BYTES as u64 + 1));

            reader.read_until(b'\n', &mut reply).await?;

            if reply.len() > MAX_MESSAGE_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "control response too large",
                ));
            }

            Ok(serde_json::from_slice(&reply)?)
        };

        // Poll the deadline first so expiry wins when I/O is ready simultaneously.
        match select(Box::pin(Timer::at(deadline)), Box::pin(exchange)).await {
            Either::Right((result, _)) if Instant::now() < deadline => result,
            _ => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "control exchange timed out",
            )),
        }
    })
}
