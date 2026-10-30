//! Owned worker process tree; tools inherit its process group or Windows Job.
use command_group::{CommandGroup, GroupChild};
use std::{
    io,
    process::{Command, ExitStatus, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

/// Owns a worker and its descendants until completion or cancellation.
pub struct Worker {
    child: GroupChild,
    finished: bool,
}

impl Worker {
    /// Starts a worker with inherited diagnostics and no stdin.
    ///
    /// # Errors
    /// Returns the OS process/job creation error.
    pub fn spawn(command: &mut Command) -> io::Result<Self> {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        Self::spawn_with_io(command)
    }

    pub(crate) fn spawn_with_io(command: &mut Command) -> io::Result<Self> {
        Ok(Self {
            child: command.group_spawn()?,
            finished: false,
        })
    }

    /// Waits with a deadline and a caller-owned cancellation flag.
    ///
    /// Cancellation kills the entire process tree and reaps the worker.
    /// # Errors
    /// Returns interruption, timeout or OS wait errors.
    pub fn wait(&mut self, cancelled: &AtomicBool, timeout: Duration) -> io::Result<ExitStatus> {
        self.wait_inner(cancelled, timeout, None)
    }

    pub(crate) fn take_stdout(&mut self) -> Option<std::process::ChildStdout> {
        self.child.inner().stdout.take()
    }
    pub(crate) fn wait_with_abort(
        &mut self,
        cancelled: &AtomicBool,
        timeout: Duration,
        abort: &AtomicBool,
    ) -> io::Result<ExitStatus> {
        self.wait_inner(cancelled, timeout, Some(abort))
    }
    fn wait_inner(
        &mut self,
        cancelled: &AtomicBool,
        timeout: Duration,
        abort: Option<&AtomicBool>,
    ) -> io::Result<ExitStatus> {
        if self.finished {
            return self.child.wait();
        }
        let started = Instant::now();
        loop {
            if abort.is_some_and(|abort| abort.load(Ordering::Acquire)) {
                self.cancel()?;
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "distribution worker output exceeded bound",
                ));
            }
            if cancelled.load(Ordering::Acquire) || started.elapsed() >= timeout {
                self.cancel()?;
                return Err(io::Error::new(
                    if cancelled.load(Ordering::Acquire) {
                        io::ErrorKind::Interrupted
                    } else {
                        io::ErrorKind::TimedOut
                    },
                    "distribution worker stopped",
                ));
            }
            if let Some(status) = self.child.try_wait()? {
                // Kill descendants even when an unsuccessful tool abandoned children.
                let _ = self.child.kill();
                self.finished = true;
                return Ok(status);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// Stops and reaps this worker tree. Repeated cancellation is harmless.
    /// # Errors
    /// Returns an OS wait error; termination of an already exited group is ignored.
    pub fn cancel(&mut self) -> io::Result<()> {
        if !self.finished {
            let _ = self.child.kill();
            self.child.wait()?;
            self.finished = true;
        }
        Ok(())
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.cancel();
    }
}
