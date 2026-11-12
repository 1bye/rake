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
