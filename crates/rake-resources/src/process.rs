//! Owned native helpers are terminated and reaped when their task owner is dropped.
use command_group::{CommandGroup, GroupChild};
use std::{
    io,
    process::{ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus},
};

/// Owns a helper's process group (Unix) or Job Object (Windows), including descendants.
/// Dropping the owner stops the entire tree and reaps its root process. Blocking
/// waits belong on an application-owned worker, away from the foreground executor.
pub struct HelperProcess {
    child: GroupChild,
    finished: bool,
}
impl std::fmt::Debug for HelperProcess {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HelperProcess")
            .field("pid", &self.child.id())
            .field("finished", &self.finished)
            .finish()
    }
}
impl HelperProcess {
    /// Starts an explicitly configured helper command with owned process-tree cleanup.
    /// Use [`crate::Resources::gstreamer_command`] to prepare private plugin paths.
    /// Stdio and environment retain the caller's configuration.
    /// # Errors
    /// Returns process/Job creation failures without detaching a helper.
    pub fn spawn(mut command: Command) -> io::Result<Self> {
        Ok(Self {
            child: command.group_spawn()?,
            finished: false,
        })
    }
    /// Takes the configured piped stdin, if any. The process owner remains live.
    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.inner().stdin.take()
    }
    /// Takes the configured piped stdout, if any. Keep the owner until reads finish.
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.inner().stdout.take()
    }
    /// Takes the configured piped stderr, if any. Keep the owner until reads finish.
    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.inner().stderr.take()
    }
    /// Checks root-process completion and terminates any abandoned descendants.
    /// # Errors
    /// Returns OS wait errors. None means the helper is still running.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if self.finished {
            return self.child.try_wait();
        }
        let status = self.child.try_wait()?;
        if status.is_some() {
            let _ = self.child.kill();
            self.finished = true;
        }
        Ok(status)
    }
    /// Waits synchronously, then stops any descendants left by the root process.
    /// # Errors
    /// Returns OS wait errors; dropping after an error still attempts cleanup.
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        if self.finished {
            return self.child.wait();
        }
        let status = self.child.wait()?;
        let _ = self.child.kill();
        self.finished = true;
        Ok(status)
    }
    /// Cancels and reaps the owned tree. Repeated cancellation is harmless.
    /// # Errors
    /// Returns an OS wait failure; termination of an already-exited tree is ignored.
    pub fn cancel(&mut self) -> io::Result<()> {
        if !self.finished {
            let _ = self.child.kill();
            self.child.wait()?;
            self.finished = true;
        }
        Ok(())
    }
}
impl Drop for HelperProcess {
    fn drop(&mut self) {
        let _ = self.cancel();
    }
}
