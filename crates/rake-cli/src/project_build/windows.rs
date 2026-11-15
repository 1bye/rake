//! Windows supervisor filesystem and anonymous-pipe readiness boundaries.

use std::{
    fs, io,
    os::windows::io::AsRawHandle,
    path::{Path, PathBuf},
};
use windows_sys::Win32::{Foundation::ERROR_BROKEN_PIPE, System::Pipes::PeekNamedPipe};

pub(super) struct Directory(PathBuf);

impl Directory {
    pub(super) fn path(&self) -> &Path {
        &self.0
    }

    pub(super) fn keep(mut self) -> PathBuf {
        std::mem::take(&mut self.0)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        if !self.0.as_os_str().is_empty() {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

pub(super) fn temporary(prefix: &str) -> io::Result<Directory> {
    loop {
        let path = std::env::temp_dir().join(format!("{prefix}{}", uuid::Uuid::new_v4()));

        match rake_client::private::create_directory(&path) {
            Ok(()) => return Ok(Directory(path)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}

pub(super) fn readable(pipe: &impl AsRawHandle) -> io::Result<bool> {
    let mut bytes = 0;
    // SAFETY: pipe owns a live anonymous-pipe handle; only the byte count is queried.
    if unsafe {
        PeekNamedPipe(
            pipe.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            &mut bytes,
            std::ptr::null_mut(),
        )
    } == 0
    {
        let error = io::Error::last_os_error();

        if error.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) {
            return Ok(true);
        }

        return Err(error);
    }

    Ok(bytes != 0)
}

/// Bound the shell RPC helper without owning the editor Explorer starts.
pub(crate) fn shell_helper(
    command: &mut std::process::Command,
    deadline: std::time::Instant,
) -> super::Result<()> {
    use std::{
        sync::atomic::AtomicBool,
        time::{Duration, Instant},
    };

    if Instant::now() >= deadline {
        return Err("Windows desktop opener deadline expired".into());
    }

    let cancelled = AtomicBool::new(false);
    let mut process = super::Processes::new(&cancelled).start(command)?;

    loop {
        if let Some(status) = process.0.try_wait()? {
            return if status.success() {
                Ok(())
            } else {
                Err(format!("Windows desktop opener failed: {status}").into())
            };
        }

        if Instant::now() >= deadline {
            return Err("Windows desktop opener timed out".into());
        }

        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod shell_tests {
    use super::*;
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };

    #[test]
    fn shell_requests_expire_before_spawn_and_stop_inflight_helpers() {
        let mut expired = Command::new("rake-deliberately-missing-shell-helper");
        let error = shell_helper(&mut expired, Instant::now()).unwrap_err();

        assert!(error.to_string().contains("deadline expired"), "{error}");
        let mut blocked = Command::new("cmd");

        blocked
            .args(["/d", "/c", "ping -n 30 127.0.0.1 > nul"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let started = Instant::now();
        let error = shell_helper(&mut blocked, started + Duration::from_millis(100)).unwrap_err();

        assert!(error.to_string().contains("timed out"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
