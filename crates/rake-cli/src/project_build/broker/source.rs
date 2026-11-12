//! Exact captured-source CLI invocation; paths are resolved by the existing ctl owner.
use super::{Error, MAX_BYTES, Options, Result};
#[cfg(windows)]
use crate::project_build::Processes;
#[cfg(unix)]
use crate::project_build::processes::ProcessGroup;
use rake_http::CancellationToken;
use serde_json::Value;

use std::{
    io::Read,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::{fd::AsRawFd, unix::process::CommandExt};

pub(super) fn arguments(body: &Value) -> Result<Vec<String>> {
    let id = body["id"]
        .as_str()
        .filter(|id| {
            !id.is_empty()
                && id.len() <= 256
                && id.starts_with("element-")
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
        .ok_or_else(|| Error::new(400, "invalid exact element ID"))?;

    let default = serde_json::json!({"kind":"boundary"});
    let target = body.get("target").unwrap_or(&default);

    let object = target
        .as_object()
        .ok_or_else(|| Error::new(400, "invalid captured source target"))?;

    let mut args = vec![id.into()];

    match target["kind"].as_str() {
        Some("boundary") => {
            if object.len() != 1 {
                return Err(Error::new(400, "invalid boundary source target"));
            }
        }
        Some(kind @ ("style" | "handler")) => {
            let index = target["index"].as_u64().filter(|index| *index < 256);

            if object.len() != 2 || index.is_none() {
                return Err(Error::new(400, "invalid captured source index"));
            }

            args.extend([
                format!("--{kind}"),
                index.expect("validated index").to_string(),
            ]);
        }
        _ => return Err(Error::new(400, "invalid captured source target")),
    }

    Ok(args)
}

pub(super) fn open(
    options: &Options,
    runtime: &str,
    args: &[String],
    cancelled: &CancellationToken,
) -> Result<Value> {
    let unavailable = || {
        Error::new(
            502,
            "Source opener unavailable; check RAKE_CLI and RAKE_SOURCE_ROOT",
        )
    };

    if cancelled.is_cancelled() {
        return Err(unavailable());
    }

    let mut command = Command::new(&options.cli);

    command
        .args(["ctl", "--instance", runtime, "inspect", "source"])
        .args(args)
        .arg("--json")
        .env("RAKE_CONTROL_DIR", &options.directory)
        .env("RAKE_SOURCE_ROOT", &options.source_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    let mut group = ProcessGroup(
        command
            .process_group(0)
            .spawn()
            .map_err(|_| unavailable())?,
    );

    #[cfg(windows)]
    let mut group = Processes::new(&std::sync::atomic::AtomicBool::new(false))
        .start(&mut command)
        .map_err(|_| unavailable())?;

    let mut stdout = group.0.stdout.take().expect("source stdout was piped");
    let mut stderr = group.0.stderr.take().expect("source stderr was piped");

    #[cfg(unix)]
    for fd in [stdout.as_raw_fd(), stderr.as_raw_fd()] {
        // SAFETY: both descriptors belong to live owned pipes. Flags preserve their mode.
        let result = unsafe { libc::fcntl(fd, libc::F_GETFL) };

        if result < 0 {
            return Err(unavailable());
        }

        // SAFETY: setting O_NONBLOCK on a live owned descriptor retains all prior flags.
        if unsafe { libc::fcntl(fd, libc::F_SETFL, result | libc::O_NONBLOCK) } < 0 {
            return Err(unavailable());
        }
    }

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut output = Vec::new();
    let mut errors = Vec::new();
    let mut output_done = false;
    let mut errors_done = false;

    loop {
        if cancelled.is_cancelled() || Instant::now() >= deadline {
            return Err(unavailable());
        }

        output_done |= drain(&mut stdout, &mut output).map_err(|_| unavailable())?;
        errors_done |= drain(&mut stderr, &mut errors).map_err(|_| unavailable())?;

        if let Some(status) = group.0.try_wait().map_err(|_| unavailable())? {
            if !status.success() {
                let message = String::from_utf8_lossy(&errors)
                    .chars()
                    .take(2048)
                    .collect::<String>();

                return Err(Error::new(
                    400,
                    if message.is_empty() {
                        "Cannot open source".into()
                    } else {
                        message
                    },
                ));
            }

            if output_done && errors_done {
                return serde_json::from_slice(&output).map_err(|_| unavailable());
            }
        }

        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(unix)]
fn drain(pipe: &mut impl Read, bytes: &mut Vec<u8>) -> std::io::Result<bool> {
    let mut buffer = [0; 8192];

    loop {
        match pipe.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(count) => {
                if bytes.len() + count > MAX_BYTES {
                    return Err(std::io::Error::other("source output exceeds 1 MiB"));
                }

                bytes.extend_from_slice(&buffer[..count]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

#[cfg(windows)]
fn drain(
    pipe: &mut (impl Read + std::os::windows::io::AsRawHandle),
    bytes: &mut Vec<u8>,
) -> std::io::Result<bool> {
    let mut buffer = [0; 8192];

    while super::super::windows::readable(pipe)? {
        match pipe.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(count) => {
                if bytes.len() + count > MAX_BYTES {
                    return Err(std::io::Error::other("source output exceeds 1 MiB"));
                }

                bytes.extend_from_slice(&buffer[..count]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }

    Ok(false)
}
