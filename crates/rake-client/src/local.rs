//! Owner-only Unix discovery and bounded blocking socket exchanges.

pub use crate::{Endpoint, Instance};
use rake_protocol::control::{ControlError, MAX_MESSAGE_BYTES, Reply, Request};
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

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
