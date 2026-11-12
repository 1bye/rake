//! Nonblocking local byte pipes. No worker or pending kernel I/O survives cancellation.

use crate::private::Security;
use async_io::Timer;
use futures::io::{AsyncRead, AsyncWrite};
use std::{
    fs::File,
    future::Future,
    io::{self, Read, Write},
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle},
    },
    path::{Path, PathBuf},
    pin::Pin,
    sync::Mutex,
    task::{Context, Poll},
    time::Duration,
};
use windows_sys::Win32::{Foundation::*, Storage::FileSystem::*, System::Pipes::*};

const POLL_INTERVAL: Duration = Duration::from_millis(5);

fn name(path: &Path) -> io::Result<Vec<u16>> {
    let text = path.to_str().ok_or(io::ErrorKind::InvalidInput)?;

    // Never connect to a remote pipe, a device, or another pipe namespace.
    if !text.starts_with(r"\\.\pipe\rake-")
        || !text[14..]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        || text.len() > 240
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid local Rake pipe name",
        ));
    }

    Ok(path.as_os_str().encode_wide().chain(Some(0)).collect())
}

/// Returns the local pipe name for a UUID runtime identity.
#[must_use]
pub fn path(runtime: &impl std::fmt::Display) -> PathBuf {
    PathBuf::from(format!(r"\\.\pipe\rake-{runtime}"))
}

/// Owns a nonblocking byte pipe, polled cooperatively at a bounded interval.
pub struct Stream {
    file: File,
    read_timer: Option<Pin<Box<Timer>>>,
    write_timer: Option<Pin<Box<Timer>>>,
}

impl Stream {
    fn new(file: File) -> Self {
        Self {
            file,
            read_timer: None,
            write_timer: None,
        }
    }

    /// Connects without blocking the executor; dropping the future cancels retries.
    ///
    /// # Errors
    /// Returns invalid-name, access, or other pipe connection failures.
    pub async fn connect(path: &Path) -> io::Result<Self> {
        let name = name(path)?;

        loop {
            // SECURITY_IDENTIFICATION prevents a pipe server from impersonating the client.
            // SAFETY: terminated name, null security/template, flags valid for a pipe.
            let handle = unsafe {
                CreateFileW(
                    name.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE | READ_CONTROL,
                    0,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                    std::ptr::null_mut(),
                )
            };

            if handle != INVALID_HANDLE_VALUE {
                // SAFETY: CreateFileW transferred ownership of a valid handle.
                let file = unsafe { File::from_raw_handle(handle) };
                crate::private::validate_security(file.as_raw_handle())?;
                let mode = PIPE_NOWAIT | PIPE_READMODE_BYTE;
                // SAFETY: live pipe, valid mode pointer, other settings unchanged.
                if unsafe {
                    SetNamedPipeHandleState(
                        file.as_raw_handle(),
                        &mode,
                        std::ptr::null(),
                        std::ptr::null(),
                    )
                } == 0
                {
                    return Err(io::Error::last_os_error());
                }

                return Ok(Self::new(file));
            }

            let error = io::Error::last_os_error();

            if !matches!(error.raw_os_error(), Some(code) if code == ERROR_PIPE_BUSY as i32 || code == ERROR_FILE_NOT_FOUND as i32)
            {
                return Err(error);
            }

            Timer::after(POLL_INTERVAL).await;
        }
    }
}

fn retry(timer: &mut Option<Pin<Box<Timer>>>, cx: &mut Context<'_>) -> Poll<io::Result<usize>> {
    let timer = timer.get_or_insert_with(|| Box::pin(Timer::after(POLL_INTERVAL)));

    if timer.as_mut().poll(cx).is_ready() {
        timer.as_mut().set(Timer::after(POLL_INTERVAL));
        let _ = timer.as_mut().poll(cx);
    }

    Poll::Pending
}

impl AsyncRead for Stream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        match self.file.read(buffer) {
            Err(error) if error.raw_os_error() == Some(ERROR_NO_DATA as i32) => {
                retry(&mut self.read_timer, cx)
            }
            Err(error) if error.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) => {
                Poll::Ready(Ok(0))
            }
            result => Poll::Ready(result),
        }
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.file.write(buffer) {
            Ok(0) if !buffer.is_empty() => retry(&mut self.write_timer, cx),
            Err(error) if error.raw_os_error() == Some(ERROR_NO_DATA as i32) => {
                retry(&mut self.write_timer, cx)
            }
            result => Poll::Ready(result),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // FlushFileBuffers waits for peer reads; byte writes are already visible.
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// Owns the listening pipe instance. Accepted streams own their individual handles.
pub struct Listener {
    path: PathBuf,
    pending: Mutex<File>,
}

fn instance(path: &Path, first: bool) -> io::Result<File> {
    let name = name(path)?;
    let security = Security::current_user()?;
    let attributes = security.attributes();
    // SAFETY: name and descriptor are live; NOWAIT avoids uncancellable blocking I/O.
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX
                | if first {
                    FILE_FLAG_FIRST_PIPE_INSTANCE
                } else {
                    0
                },
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
            33,
            65536,
            65536,
            0,
            &attributes,
        )
    };

    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: CreateNamedPipeW transferred ownership of a valid handle.
    Ok(unsafe { File::from_raw_handle(handle) })
}

impl Listener {
    /// Exclusively binds a current-user-only pipe, rejecting remote clients.
    ///
    /// # Errors
    /// Returns pipe creation errors, including an existing first instance.
    pub fn bind(path: &Path) -> io::Result<Self> {
        Ok(Self {
            path: path.into(),
            pending: Mutex::new(instance(path, true)?),
        })
    }

    /// Accepts one client; cancellation leaves the listener usable.
    ///
    /// # Errors
    /// Returns pipe connection or replacement-instance creation failures.
    pub async fn accept(&self) -> io::Result<Stream> {
        loop {
            {
                let mut pending = self.pending.lock().expect("pipe listener lock poisoned");
                // SAFETY: pending is a live NOWAIT server handle; no OVERLAPPED needed.
                let _connected =
                    unsafe { ConnectNamedPipe(pending.as_raw_handle(), std::ptr::null_mut()) };
                let error = io::Error::last_os_error();

                let mut peer = 0;
                // NOWAIT ConnectNamedPipe can succeed while merely making an instance
                // available. A client PID confirms that a client actually connected.
                // SAFETY: pending is live and peer is a valid output slot.
                let has_peer =
                    unsafe { GetNamedPipeClientProcessId(pending.as_raw_handle(), &mut peer) } != 0;

                if has_peer {
                    let replacement = instance(&self.path, false)?;
                    let accepted = std::mem::replace(&mut *pending, replacement);

                    return Ok(Stream::new(accepted));
                }

                if _connected == 0
                    && !matches!(error.raw_os_error(), Some(code) if code == ERROR_PIPE_LISTENING as i32 || code == ERROR_NO_DATA as i32 || code == ERROR_PIPE_CONNECTED as i32)
                {
                    return Err(error);
                }

                if error.raw_os_error() == Some(ERROR_NO_DATA as i32) {
                    // SAFETY: this server instance has a disconnected client and no I/O pending.
                    unsafe {
                        DisconnectNamedPipe(pending.as_raw_handle());
                    }
                }
            }

            Timer::after(POLL_INTERVAL).await;
        }
    }
}
