//! Keep protocol fixture assertions identical across local endpoint transports.

use std::{
    io,
    path::{Path, PathBuf},
};

#[cfg(windows)]
pub(crate) use pipes::{Listener, Stream};
#[cfg(unix)]
pub(crate) use std::os::unix::net::{UnixListener as Listener, UnixStream as Stream};

pub(crate) fn path(unix: String) -> PathBuf {
    #[cfg(unix)]
    {
        unix.into()
    }

    #[cfg(windows)]
    {
        let _ = unix;

        rake_client::pipe::path(&uuid::Uuid::new_v4())
    }
}

pub(crate) fn remove(path: impl AsRef<Path>) -> io::Result<()> {
    #[cfg(windows)]
    if path
        .as_ref()
        .to_string_lossy()
        .starts_with(r"\\.\pipe\rake-")
    {
        return Ok(());
    }

    std::fs::remove_file(path)
}

#[cfg(windows)]
mod pipes {
    use super::*;
    use futures::io::{AsyncRead, AsyncWrite};
    use std::{
        future::Future,
        io::{Read, Write},
        pin::Pin,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
        task::{Context, Poll},
        time::{Duration, Instant},
    };

    fn wait<T>(
        nonblocking: bool,
        timeout: Duration,
        mut poll: impl FnMut(&mut Context<'_>) -> Poll<io::Result<T>>,
    ) -> io::Result<T> {
        let deadline = Instant::now() + timeout;
        let mut context = Context::from_waker(futures::task::noop_waker_ref());

        loop {
            if let Poll::Ready(result) = poll(&mut context) {
                return result;
            }
            if nonblocking {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            if Instant::now() >= deadline {
                return Err(io::ErrorKind::TimedOut.into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    pub(crate) struct Listener {
        inner: Arc<rake_client::pipe::Listener>,
        nonblocking: Arc<AtomicBool>,
    }

    impl Listener {
        pub(crate) fn bind(path: &Path) -> io::Result<Self> {
            Ok(Self {
                inner: Arc::new(rake_client::pipe::Listener::bind(path)?),
                nonblocking: Arc::new(AtomicBool::new(false)),
            })
        }
        pub(crate) fn try_clone(&self) -> io::Result<Self> {
            Ok(Self {
                inner: self.inner.clone(),
                nonblocking: self.nonblocking.clone(),
            })
        }
        pub(crate) fn set_nonblocking(&self, value: bool) -> io::Result<()> {
            self.mode().store(value, Ordering::Relaxed);
            Ok(())
        }
        pub(crate) fn accept(&self) -> io::Result<(Stream, ())> {
            let future = self.inner.accept();
            let mut future = std::pin::pin!(future);
            let stream = wait(
                self.mode().load(Ordering::Relaxed),
                Duration::from_secs(3),
                |cx| future.as_mut().poll(cx),
            )?;
            Ok((Stream::new(stream), ()))
        }
        fn mode(&self) -> &AtomicBool {
            &self.nonblocking
        }
    }

    pub(crate) struct Stream {
        inner: Arc<Mutex<rake_client::pipe::Stream>>,
        nonblocking: Arc<AtomicBool>,
        timeout: Arc<Mutex<Duration>>,
        written: Arc<AtomicBool>,
    }

    impl std::fmt::Debug for Stream {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("PipeFixtureStream")
        }
    }

    impl Stream {
        fn new(stream: rake_client::pipe::Stream) -> Self {
            Self {
                inner: Arc::new(Mutex::new(stream)),
                nonblocking: Arc::new(AtomicBool::new(false)),
                timeout: Arc::new(Mutex::new(Duration::from_secs(3))),
                written: Arc::new(AtomicBool::new(false)),
            }
        }
        pub(crate) fn connect(path: &Path) -> io::Result<Self> {
            let future = rake_client::pipe::Stream::connect(path);
            let mut future = std::pin::pin!(future);
            wait(false, Duration::from_secs(3), |cx| future.as_mut().poll(cx)).map(Self::new)
        }
        pub(crate) fn try_clone(&self) -> io::Result<Self> {
            Ok(Self {
                inner: self.inner.clone(),
                nonblocking: self.nonblocking.clone(),
                timeout: self.timeout.clone(),
                written: self.written.clone(),
            })
        }
        pub(crate) fn set_nonblocking(&self, value: bool) -> io::Result<()> {
            self.nonblocking.store(value, Ordering::Relaxed);
            Ok(())
        }
        pub(crate) fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
            *self.timeout.lock().unwrap() = timeout.unwrap_or(Duration::from_secs(3));
            Ok(())
        }
        fn poll<T>(
            &self,
            mut operation: impl FnMut(
                Pin<&mut rake_client::pipe::Stream>,
                &mut Context<'_>,
            ) -> Poll<io::Result<T>>,
        ) -> io::Result<T> {
            let timeout = *self.timeout.lock().unwrap();
            wait(self.nonblocking.load(Ordering::Relaxed), timeout, |cx| {
                operation(Pin::new(&mut *self.inner.lock().unwrap()), cx)
            })
        }
    }

    impl Read for Stream {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            self.poll(|stream, cx| stream.poll_read(cx, bytes))
        }
    }
    impl Write for Stream {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let count = self.poll(|stream, cx| stream.poll_write(cx, bytes))?;
            self.written.store(true, Ordering::Relaxed);
            Ok(count)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.poll(|stream, cx| stream.poll_flush(cx))
        }
    }
    impl Drop for Stream {
        fn drop(&mut self) {
            if Arc::strong_count(&self.inner) == 1 && self.written.load(Ordering::Relaxed) {
                // A server pipe discards queued bytes on close; let the bounded
                // production client consume its frame and close first.
                let _ = self.read(&mut [0]);
            }
        }
    }
}
