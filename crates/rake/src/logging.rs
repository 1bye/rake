//! Bounded private application logs, written only by the diagnostics worker.

#[cfg(any(not(target_family = "wasm"), feature = "gpui", test))]
use std::path::PathBuf;
#[cfg(any(not(target_family = "wasm"), feature = "gpui", test))]
use std::sync::OnceLock;

#[cfg(any(not(target_family = "wasm"), feature = "gpui", test))]
static PATH: OnceLock<PathBuf> = OnceLock::new();

/// Configuration for automatic application logging.
#[derive(Clone, Debug, Default)]
#[must_use]
pub struct LogConfig {
    /// Default severity; a valid `RAKE_LOG` environment value takes precedence.
    /// If absent, the application's `log-level` preference is used, then INFO.
    pub level: Option<String>,
}

#[cfg(any(not(target_family = "wasm"), feature = "gpui", test))]
pub(crate) fn path() -> Option<String> {
    PATH.get().map(|path| path.to_string_lossy().into_owned())
}

#[cfg(all(not(target_family = "wasm"), any(feature = "gpui", test)))]
mod native {
    use super::PATH;
    use crate::storage::file::{check_directory, check_file, private_options};
    use std::fs;
    use std::io::{self, Write};
    use std::path::{Path, PathBuf};

    // Four files of at most 1 MiB: a hard 4 MiB per-application budget.
    const FILE_BYTES: u64 = 1024 * 1024;
    const ROTATIONS: usize = 3;

    #[cfg(feature = "gpui")]
    pub(super) fn directory(id: &str) -> Option<PathBuf> {
        if id.is_empty()
            || id.len() > 128
            || matches!(id, "." | "..")
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return None;
        }

        #[cfg(target_os = "macos")]
        let root = std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Logs"));
        #[cfg(target_os = "windows")]
        let root = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let root = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
            });

        root.filter(|root| root.is_absolute())
            .map(|root| root.join(id).join("logs"))
    }

    struct LogWriter {
        directory: PathBuf,
        _lock: fs::File,
        pending: Vec<u8>,
    }

    impl LogWriter {
        fn open(directory: PathBuf) -> io::Result<Self> {
            let mut builder = fs::DirBuilder::new();

            builder.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }

            builder.create(&directory)?;
            check_directory(&directory)?;
            let lock = private_options()
                .write(true)
                .create(true)
                .truncate(false)
                .open(directory.join("writer.lock"))?;

            check_file(&lock)?;
            lock.try_lock().map_err(io::Error::other)?;

            // Validate every managed file before any rotation or truncation.
            for index in 0..=ROTATIONS {
                let path = file_path(&directory, index);

                if path.try_exists()? {
                    let file = private_options().read(true).open(&path)?;
                    check_file(&file)?;

                    if file.metadata()?.len() > FILE_BYTES {
                        return Err(io::Error::other("oversized application log"));
                    }
                }
            }

            let file = private_options()
                .append(true)
                .create(true)
                .open(file_path(&directory, 0))?;

            check_file(&file)?;

            Ok(Self {
                directory,
                _lock: lock,
                pending: Vec::new(),
            })
        }

        fn line(&self, bytes: &[u8]) -> io::Result<()> {
            check_directory(&self.directory)?;
            let path = file_path(&self.directory, 0);
            let mut file = private_options().append(true).create(true).open(&path)?;
            check_file(&file)?;

            if file.metadata()?.len().saturating_add(bytes.len() as u64) > FILE_BYTES {
                drop(file);

                for index in (1..=ROTATIONS).rev() {
                    let old = file_path(&self.directory, index);

                    match fs::remove_file(&old) {
                        Ok(()) => {}
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error),
                    }

                    let previous = file_path(&self.directory, index - 1);

                    match fs::rename(previous, old) {
                        Ok(()) => {}
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error),
                    }
                }

                file = private_options().append(true).create_new(true).open(path)?;
            }

            file.write_all(bytes)
        }
    }

    impl Drop for LogWriter {
        fn drop(&mut self) {
            // Explicit unlock also releases a descriptor inherited across fork.
            let _ = self._lock.unlock();
        }
    }

    fn file_path(directory: &Path, index: usize) -> PathBuf {
        directory.join(if index == 0 {
            "app.log".into()
        } else {
            format!("app.log.{index}")
        })
    }

    impl Write for LogWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            for &byte in bytes {
                // Reserve the last byte for framing even if a record is oversized.
                if byte == b'\n' {
                    self.pending.push(byte);
                    let _ = self.line(&self.pending);
                    self.pending.clear();
                } else if self.pending.len() < 255 {
                    self.pending.push(byte);
                }
            }

            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    pub(super) fn writer(directory: PathBuf) -> Box<dyn Write + Send> {
        match LogWriter::open(directory) {
            Ok(writer) => {
                let _ = PATH.set(writer.directory.join("app.log"));
                Box::new(writer)
            }
            Err(_) => Box::new(io::sink()),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn rotation_bounds_and_exclusive_writer() {
            let temporary = tempfile::tempdir().expect("temporary logs");
            let directory = temporary.path().join("logs");
            let mut writer = LogWriter::open(directory.clone()).expect("private writer");
            assert!(LogWriter::open(directory.clone()).is_err());

            let line = vec![b'x'; 256];

            for _ in 0..(FILE_BYTES as usize * 5 / line.len()) {
                writer.line(&line).expect("rotate");
            }

            writer.write_all(b"INFO event\n").expect("write");
            let sizes: Vec<_> = (0..=ROTATIONS)
                .map(|index| {
                    fs::metadata(file_path(&directory, index))
                        .expect("retained log")
                        .len()
                })
                .collect();

            assert!(sizes.iter().all(|size| *size <= FILE_BYTES));
            assert!(sizes.iter().sum::<u64>() <= FILE_BYTES * 4);
            assert!(
                fs::read_to_string(file_path(&directory, 0))
                    .expect("tail")
                    .ends_with("INFO event\n")
            );
        }

        #[test]
        fn failing_directory_is_a_sink() {
            let temporary = tempfile::tempdir().expect("temporary logs");
            let file = temporary.path().join("file");

            fs::write(&file, b"occupied").expect("fixture");
            writer(file)
                .write_all(b"INFO event\n")
                .expect("failure is harmless");

            let directory = temporary.path().join("logs");
            let mut writer = LogWriter::open(directory.clone()).expect("writer");
            fs::remove_file(directory.join("app.log")).expect("remove empty log");
            fs::create_dir(directory.join("app.log")).expect("obstruct log file");
            writer
                .write_all(b"INFO event\n")
                .expect("write failure is harmless");
        }
    }
}

#[cfg(feature = "gpui")]
pub(crate) fn configure(
    mut diagnostics: crate::diagnostics::DiagnosticsConfig,
    id: &str,
    config: LogConfig,
) -> crate::diagnostics::DiagnosticsConfig {
    if let Some(level) = config.level.and_then(|level| level.parse().ok()) {
        diagnostics = diagnostics.log_level(level);
    }

    #[cfg(not(target_family = "wasm"))]
    if !cfg!(debug_assertions)
        && let Some(directory) = native::directory(id)
    {
        diagnostics = diagnostics.log_writer(move || native::writer(directory.clone()));
    }

    #[cfg(target_family = "wasm")]
    let _ = id;

    diagnostics
}
