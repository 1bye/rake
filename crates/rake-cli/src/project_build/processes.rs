//! Bounded tool output and owned process trees, including compiler descendants.

use super::Result;

use std::{
    io::{Read, Write},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

#[cfg(unix)]
use std::{
    os::{
        fd::OwnedFd,
        unix::{net::UnixStream, process::CommandExt},
    },
    time::Instant,
};

#[cfg(windows)]
mod windows;

const MAX_BYTES: usize = 1024 * 1024;
const METADATA_LIMIT: usize = 32 * MAX_BYTES;

pub(super) enum Capture {
    Metadata,
    Version,
    Diagnostic,
}

type OutputSink<'a> = dyn Fn(&str, u32, &str) + Sync + 'a;
type DiagnosticSink<'a> = dyn Fn(&str) -> std::io::Result<()> + Sync + 'a;

pub(super) struct Processes<'a> {
    cancelled: &'a AtomicBool,
    stopping: Option<&'a AtomicBool>,
    output_sink: Option<&'a OutputSink<'a>>,
    diagnostic_sink: Option<&'a DiagnosticSink<'a>>,
}

impl<'a> Processes<'a> {
    pub(super) fn new(cancelled: &'a AtomicBool) -> Self {
        Self {
            cancelled,
            stopping: None,
            output_sink: None,
            diagnostic_sink: None,
        }
    }

    pub(super) fn with_stopping(mut self, stopping: &'a AtomicBool) -> Self {
        self.stopping = Some(stopping);

        self
    }

    pub(super) fn with_output(mut self, sink: &'a OutputSink<'a>) -> Self {
        self.output_sink = Some(sink);

        self
    }

    pub(super) fn with_diagnostics(mut self, sink: Option<&'a DiagnosticSink<'a>>) -> Self {
        self.diagnostic_sink = sink;

        self
    }

    pub(super) fn start(&self, command: &mut Command) -> Result<ProcessGroup> {
        self.check_cancelled()?;
        #[cfg(unix)]
        {
            Ok(ProcessGroup(command.process_group(0).spawn()?))
        }

        #[cfg(windows)]
        {
            windows::spawn(command)
        }
    }

    pub(super) fn capture(&self, command: &mut Command, mode: Capture) -> Result<String> {
        let mut bytes = Vec::new();
        let mut diagnostic = Vec::new();

        let status = self.execute(
            command,
            matches!(mode, Capture::Diagnostic),
            |chunk, pid, stderr| {
                if stderr {
                    diagnostic.extend_from_slice(chunk);

                    while let Some(end) = diagnostic
                        .iter()
                        .position(|byte| *byte == b'\n')
                        .map(|index| index + 1)
                        .or((diagnostic.len() >= MAX_BYTES).then_some(MAX_BYTES))
                    {
                        self.diagnostic(&String::from_utf8_lossy(&diagnostic[..end]), pid)?;
                        diagnostic.drain(..end);
                    }

                    return Ok(());
                }

                if bytes.len() + chunk.len() > METADATA_LIMIT {
                    return Err("tool metadata exceeds 32 MiB".into());
                }

                bytes.extend_from_slice(chunk);

                Ok(())
            },
        )?;

        if !diagnostic.is_empty() {
            self.diagnostic(&String::from_utf8_lossy(&diagnostic), 0)?;
        }

        if !status.success() {
            return Err(if !matches!(mode, Capture::Metadata) {
                format!(
                    "Command '{}' returned non-zero exit status {}.",
                    diagnostic_command(command),
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
        let mut tail: Vec<String> = Vec::new();

        let blaze = std::path::Path::new(command.get_program())
            .file_name()
            .is_some_and(|name| name == "blaze" || name == "blaze-engine");

        let mut structured_error = None;
        let mut diagnostic_tail = None;

        let mut line = |bytes: &[u8], pid| -> Result<()> {
            let text = String::from_utf8_lossy(bytes).replace("\r\n", "\n");
            let message = serde_json::from_str::<serde_json::Value>(&text).ok();

            if blaze {
                if let Some(finished) = message
                    .as_ref()
                    .and_then(|message| message["event"].get("Finished"))
                    && finished["ok"] == false
                {
                    structured_error = finished["summary"]["error"].as_str().map(str::to_owned);
                }

                if text.starts_with("blaze time:") && diagnostic_tail.is_none() {
                    diagnostic_tail = Some(last_chars(&tail.concat(), 8000));
                }
            }

            let printed = match message {
                Some(message) if message.is_object() => {
                    if message["reason"] == "compiler-artifact" {
                        artifact(message);

                        None
                    } else if message["reason"] == "compiler-message" {
                        Some(
                            message["message"]["rendered"]
                                .as_str()
                                .unwrap_or("")
                                .to_owned(),
                        )
                    } else {
                        None
                    }
                }
                _ => Some(text),
            };

            if let Some(text) = printed {
                if let Some(sink) = self.output_sink {
                    sink(&text, pid, "build");
                }

                if !text.is_empty() {
                    self.print_diagnostic(&text)?;
                    tail.push(last_chars(&text, 4000));

                    if tail.len() > 8 {
                        tail.remove(0);
                    }
                }
            }

            Ok(())
        };

        let mut output_pid = 0;

        let status = self.execute(command, true, |chunk, pid, _| {
            output_pid = pid;
            pending.extend_from_slice(chunk);

            while let Some(end) = pending
                .iter()
                .take(MAX_BYTES)
                .position(|byte| *byte == b'\n')
                .map(|index| index + 1)
                .or((pending.len() >= MAX_BYTES).then_some(MAX_BYTES))
            {
                line(&pending[..end], pid)?;
                pending.drain(..end);
            }

            Ok(())
        })?;

        if !pending.is_empty() {
            line(&pending, output_pid)?;
        }

        if !status.success() {
            return Err(format!(
                "{} exited {}\n{}",
                command.get_program().to_string_lossy(),
                exit_code(status),
                structured_error
                    .or(diagnostic_tail)
                    .unwrap_or_else(|| last_chars(&tail.concat(), 8000))
            )
            .into());
        }

        Ok(())
    }

    fn diagnostic(&self, text: &str, pid: u32) -> std::io::Result<()> {
        if let Some(sink) = self.output_sink {
            sink(text, pid, "build");
        }

        self.print_diagnostic(text)
    }

    fn print_diagnostic(&self, text: &str) -> std::io::Result<()> {
        if let Some(sink) = self.diagnostic_sink {
            return sink(text);
        }

        let mut stderr = std::io::stderr().lock();

        stderr.write_all(text.as_bytes())?;

        stderr.flush()
    }

    pub(super) fn check_cancelled(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Acquire)
            || self
                .stopping
                .is_some_and(|flag| flag.load(Ordering::Acquire))
        {
            return Err("build interrupted".into());
        }

        Ok(())
    }

    fn execute(
        &self,
        command: &mut Command,
        merge_stderr: bool,
        mut consume: impl FnMut(&[u8], u32, bool) -> Result<()>,
    ) -> Result<ExitStatus> {
        self.check_cancelled()?;
        let (mut reader, stdout, stderr) = output_pipe(merge_stderr)?;

        let diagnostic_pipe = if !merge_stderr && self.diagnostic_sink.is_some() {
            Some(output_pipe(false)?)
        } else {
            None
        };

        command.stdout(stdout);
        let mut diagnostic_reader = None;

        command.stderr(if let Some((reader, writer, _)) = diagnostic_pipe {
            diagnostic_reader = Some(reader);

            writer
        } else {
            stderr.unwrap_or_else(Stdio::inherit)
        });
        let mut group = self.start(command).map_err(|error| {
            // Retain the missing-tool diagnostic used by doctor and build callers.
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound)
            {
                format!(
                    "[Errno 2] No such file or directory: '{}'",
                    command.get_program().to_string_lossy()
                )
            } else {
                format!("{}: {error}", command.get_program().to_string_lossy())
            }
        })?;

        command.stdout(Stdio::null()).stderr(Stdio::inherit());
        let (sender, receiver) = mpsc::sync_channel(4);

        let diagnostic_thread = diagnostic_reader.map(|mut reader| {
            let sender = sender.clone();

            std::thread::spawn(move || {
                let mut buffer = [0; 8192];

                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(count) => {
                            if sender.send(Ok((buffer[..count].to_vec(), true))).is_err() {
                                break;
                            }
                        }
                        Err(error) => {
                            let _ = sender.send(Err(error));

                            break;
                        }
                    }
                }
            })
        });

        let thread = std::thread::spawn(move || {
            let mut buffer = [0; 8192];

            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        if sender.send(Ok((buffer[..count].to_vec(), false))).is_err() {
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
                self.check_cancelled()?;

                match receiver.recv_timeout(Duration::from_millis(25)) {
                    Ok(chunk) => {
                        let (chunk, stderr) = chunk?;

                        consume(&chunk, group.0.id(), stderr)?;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            }

            loop {
                self.check_cancelled()?;

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

        if let Some(thread) = diagnostic_thread {
            thread
                .join()
                .map_err(|_| "tool diagnostic reader panicked")?;
        }

        result
    }
}

fn last_chars(text: &str, count: usize) -> String {
    text.chars()
        .skip(text.chars().count().saturating_sub(count))
        .collect()
}

fn diagnostic_command(command: &Command) -> String {
    let arguments = std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(|argument| format!("{argument:?}"))
        .collect::<Vec<_>>()
        .join(", ")
        .replace('"', "'");

    format!("[{arguments}]")
}

fn exit_code(status: ExitStatus) -> i32 {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;

        status
            .code()
            .unwrap_or_else(|| -status.signal().unwrap_or(1))
    }

    #[cfg(windows)]
    {
        status.code().unwrap_or(1)
    }
}

pub(super) struct ProcessGroup(pub(super) Child, #[cfg(windows)] windows::Job);

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
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

        #[cfg(windows)]
        {
            self.1.terminate();
            let _ = self.0.wait();
        }
    }
}

#[cfg(unix)]
fn output_pipe(merge: bool) -> std::io::Result<(UnixStream, Stdio, Option<Stdio>)> {
    let (reader, writer) = UnixStream::pair()?;
    let stdout = Stdio::from(OwnedFd::from(writer.try_clone()?));
    let stderr = merge.then(|| Stdio::from(OwnedFd::from(writer)));

    Ok((reader, stdout, stderr))
}

#[cfg(windows)]
fn output_pipe(merge: bool) -> std::io::Result<(std::fs::File, Stdio, Option<Stdio>)> {
    windows::pipe(merge)
}

pub(super) fn which(program: &str) -> Option<std::path::PathBuf> {
    let paths = std::env::var_os("PATH")?;

    #[cfg(unix)]
    let names = [std::ffi::OsString::from(program)];

    #[cfg(windows)]
    let names = {
        let mut names = vec![std::ffi::OsString::from(program)];

        if std::path::Path::new(program).extension().is_none() {
            let extensions =
                std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());

            for extension in extensions
                .split(';')
                .filter(|extension| extension.starts_with('.'))
            {
                names.push(format!("{program}{extension}").into());
            }
        }

        names
    };

    std::env::split_paths(&paths)
        .flat_map(|path| names.iter().map(move |name| path.join(name)))
        .find(|path| {
            path.metadata().is_ok_and(|metadata| {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;

                    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
                }

                #[cfg(windows)]
                {
                    metadata.is_file()
                }
            })
        })
}

#[cfg(all(test, unix))]
mod tests {
    //! Covers bounded metadata and child-command failure contracts.
    //! Real: Unix process groups and tool output pipes. Doubles: shell commands.
    //! Requires: /bin/sh and /bin/dd.
    use super::*;

    #[test]
    fn captured_native_stderr_does_not_corrupt_metadata_or_lose_partial_lines() {
        let cancelled = AtomicBool::new(false);
        let diagnostics = std::sync::Mutex::new(String::new());

        let sink = |text: &str| {
            diagnostics.lock().unwrap().push_str(text);

            Ok(())
        };

        let processes = Processes::new(&cancelled).with_diagnostics(Some(&sink));
        let output = processes.capture(Command::new("/bin/sh").args(["-c", "printf '{\"artifact\":\"app\"}'; i=0; while [ $i -lt 2000 ]; do printf 'warning: keep this diagnostic\\n' >&2; i=$((i+1)); done; printf 'last 漢字' >&2"]), Capture::Metadata).unwrap();

        assert_eq!(output, "{\"artifact\":\"app\"}");
        let diagnostics = diagnostics.lock().unwrap();

        assert_eq!(diagnostics.matches("warning: keep").count(), 2000);
        assert!(diagnostics.ends_with("last 漢字"));
    }

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
    fn relative_tool_permission_errors_are_not_reported_as_missing_programs() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let tool = root.path().join("tool");

        std::fs::write(&tool, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o600)).unwrap();
        let cancelled = AtomicBool::new(false);

        let error = Processes::new(&cancelled)
            .capture(
                Command::new("./tool").current_dir(root.path()),
                Capture::Metadata,
            )
            .unwrap_err();

        assert!(error.to_string().contains("Permission denied"), "{error}");
        assert!(!error.to_string().contains("No such file"));
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
