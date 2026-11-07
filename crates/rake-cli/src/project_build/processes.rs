//! Bounded tool output and owned Unix process groups, including compiler descendants.

use super::Result;
use std::{
    io::{Read, Write},
    os::{
        fd::OwnedFd,
        unix::{net::UnixStream, process::CommandExt},
    },
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

const MAX_BYTES: usize = 1024 * 1024;
const METADATA_LIMIT: usize = 32 * MAX_BYTES;

pub(super) enum Capture {
    Metadata,
    Version,
    Diagnostic,
}

pub(super) struct Processes<'a> {
    cancelled: &'a AtomicBool,
}

impl<'a> Processes<'a> {
    pub(super) fn new(cancelled: &'a AtomicBool) -> Self {
        Self { cancelled }
    }

    pub(super) fn capture(&self, command: &mut Command, mode: Capture) -> Result<String> {
        let mut bytes = Vec::new();
        let status = self.execute(command, matches!(mode, Capture::Diagnostic), |chunk| {
            if bytes.len() + chunk.len() > METADATA_LIMIT {
                return Err("tool metadata exceeds 32 MiB".into());
            }
            bytes.extend_from_slice(chunk);
            Ok(())
        })?;
        if !status.success() {
            return Err(if !matches!(mode, Capture::Metadata) {
                format!(
                    "Command '{}' returned non-zero exit status {}.",
                    python_command(command),
                    exit_code(status)
                )
            } else {
                format!(
                    "{} metadata command failed",
                    command.get_program().to_string_lossy()
                )
            }
            .into());
        }

        Ok(String::from_utf8(bytes)?.replace("\r\n", "\n"))
    }

    pub(super) fn run_output(
        &self,
        command: &mut Command,
        mut artifact: impl FnMut(serde_json::Value),
    ) -> Result<()> {
        let mut pending = Vec::new();
        let mut tail = Vec::new();
        let mut line = |bytes: &[u8]| -> Result<()> {
            let text = String::from_utf8_lossy(bytes).replace("\r\n", "\n");
            let message = serde_json::from_str::<serde_json::Value>(&text).ok();
            let printed = match message {
                Some(message) if message.is_object() => {
                    if message["reason"] == "compiler-artifact" {
                        artifact(message);
                        None
                    } else if message["reason"] == "compiler-message" {
                        message["message"]["rendered"]
                            .as_str()
                            .filter(|text| !text.is_empty())
                            .map(str::to_owned)
                    } else {
                        None
                    }
                }
                _ => Some(text),
            };
            if let Some(text) = printed {
                std::io::stderr().write_all(text.as_bytes())?;
                std::io::stderr().flush()?;
                tail.push(last_chars(&text, 4000));
                if tail.len() > 8 {
                    tail.remove(0);
                }
            }
            Ok(())
        };
        let status = self.execute(command, true, |chunk| {
            pending.extend_from_slice(chunk);
            while let Some(end) = pending
                .iter()
                .take(MAX_BYTES)
                .position(|byte| *byte == b'\n')
                .map(|index| index + 1)
                .or((pending.len() >= MAX_BYTES).then_some(MAX_BYTES))
            {
                line(&pending[..end])?;
                pending.drain(..end);
            }
            Ok(())
        })?;
        if !pending.is_empty() {
            line(&pending)?;
        }
        if !status.success() {
            return Err(format!(
                "{} exited {}\n{}",
                command.get_program().to_string_lossy(),
                exit_code(status),
                last_chars(&tail.concat(), 8000)
            )
            .into());
        }

        Ok(())
    }

    pub(super) fn check_cancelled(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err("build interrupted".into());
        }

        Ok(())
    }

    fn execute(
        &self,
        command: &mut Command,
        merge_stderr: bool,
        mut consume: impl FnMut(&[u8]) -> Result<()>,
    ) -> Result<ExitStatus> {
        self.check_cancelled()?;
        let (mut reader, writer) = UnixStream::pair()?;
        command
            .process_group(0)
            .stdout(Stdio::from(OwnedFd::from(writer.try_clone()?)));
        if merge_stderr {
            command.stderr(Stdio::from(OwnedFd::from(writer)));
        } else {
            command.stderr(Stdio::inherit());
            drop(writer);
        }
        let child = command.spawn().map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                format!(
                    "[Errno 2] No such file or directory: '{}'",
                    command.get_program().to_string_lossy()
                )
            } else {
                format!("{}: {error}", command.get_program().to_string_lossy())
            }
        })?;
        command.stdout(Stdio::null()).stderr(Stdio::inherit());
        let mut group = ProcessGroup(child);
        let (sender, receiver) = mpsc::sync_channel(4);
        let thread = std::thread::spawn(move || {
            let mut buffer = [0; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        if sender.send(Ok(buffer[..count].to_vec())).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = sender.send(Err(error));
                        break;
                    }
                }
            }
        });
        let result = (|| {
            loop {
                if self.cancelled.load(Ordering::Acquire) {
                    return Err("build interrupted".into());
                }
                match receiver.recv_timeout(Duration::from_millis(25)) {
                    Ok(chunk) => consume(&chunk?)?,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
            loop {
                if self.cancelled.load(Ordering::Acquire) {
                    return Err("build interrupted".into());
                }
                if let Some(status) = group.0.try_wait()? {
                    return Ok(status);
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        })();
        // Stop descendants before joining a reader which may still hold their pipe.
        drop(group);
        drop(receiver);
        thread.join().map_err(|_| "tool output reader panicked")?;

        result
    }
}

fn last_chars(text: &str, count: usize) -> String {
    text.chars()
        .skip(text.chars().count().saturating_sub(count))
        .collect()
}

fn python_command(command: &Command) -> String {
    let arguments = std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(|argument| format!("{argument:?}"))
        .collect::<Vec<_>>()
        .join(", ")
        .replace('"', "'");

    format!("[{arguments}]")
}

fn exit_code(status: ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .unwrap_or_else(|| -status.signal().unwrap_or(1))
}

struct ProcessGroup(Child);

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        let pid = self.0.id() as i32;
        // SAFETY: a negative child PID addresses the process group created above.
        unsafe {
            libc::kill(-pid, libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while self.0.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        // SAFETY: also reap descendants left behind after the group leader exited.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
        let _ = self.0.wait();
    }
}

pub(super) fn which(program: &str) -> Option<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|path| path.join(program))
            .find(|path| {
                path.metadata().is_ok_and(|metadata| {
                    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
                })
            })
    })
}

#[cfg(test)]
mod tests {
    //! Covers bounded metadata and child-command failure contracts.
    //! Real: Unix process groups and tool output pipes. Doubles: shell commands.
    //! Requires: /bin/sh and /bin/dd.
    use super::*;

    #[test]
    fn metadata_is_bounded_before_parsing() {
        let cancelled = AtomicBool::new(false);
        let processes = Processes::new(&cancelled);
        let error = processes
            .capture(
                Command::new("/bin/dd").args(["if=/dev/zero", "bs=1048576", "count=33"]),
                Capture::Metadata,
            )
            .unwrap_err();

        assert_eq!(error.to_string(), "tool metadata exceeds 32 MiB");
    }

    #[test]
    fn failed_metadata_and_diagnostic_probes_keep_their_error_contracts() {
        let cancelled = AtomicBool::new(false);
        let processes = Processes::new(&cancelled);
        let error = processes
            .capture(
                Command::new("/bin/sh").args(["-c", "exit 7"]),
                Capture::Metadata,
            )
            .unwrap_err();
        assert_eq!(error.to_string(), "/bin/sh metadata command failed");

        let error = processes
            .capture(
                Command::new("/bin/sh").args(["-c", "exit 7"]),
                Capture::Diagnostic,
            )
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Command '['/bin/sh', '-c', 'exit 7']' returned non-zero exit status 7."
        );
    }

    #[test]
    fn cancelled_work_never_starts_a_tool() {
        let cancelled = AtomicBool::new(true);
        let processes = Processes::new(&cancelled);
        let error = processes
            .capture(&mut Command::new("/nonexistent-tool"), Capture::Metadata)
            .unwrap_err();

        assert_eq!(error.to_string(), "build interrupted");
    }
}
