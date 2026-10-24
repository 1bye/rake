//! Native durability: private directory, one process lease, fsync and atomic replacement.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::{Envelope, LoadReport, MAX_BYTES, StorageError, check_write, decode, loaded};

pub(super) struct Files {
    directory: PathBuf,
    lease: Mutex<Option<File>>,
}

fn io(error: std::io::Error) -> StorageError {
    StorageError::Io(error.to_string())
}

impl Files {
    pub(super) fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            lease: Mutex::new(None),
        }
    }

    fn enter(&self) -> Result<std::sync::MutexGuard<'_, Option<File>>, StorageError> {
        let mut lease = self
            .lease
            .lock()
            .map_err(|_| StorageError::Io("Library writer stopped unexpectedly.".into()))?;

        if lease.is_none() {
            let mut builder = std::fs::DirBuilder::new();

            builder.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(&self.directory).map_err(io)?;
            let metadata = std::fs::symlink_metadata(&self.directory).map_err(io)?;

            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(StorageError::Io(
                    "Library path must be a real directory.".into(),
                ));
            }

            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;

                // SAFETY: geteuid has no preconditions.
                if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
                    return Err(StorageError::Io(
                        "Library directory must be owned by you and private (mode 0700).".into(),
                    ));
                }
            }
            let mut options = OpenOptions::new();

            options.read(true).write(true).create(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
            }
            let file = options
                .open(self.directory.join("library.lock"))
                .map_err(io)?;

            private_file(&file)?;
            #[cfg(unix)]
            {
                use std::os::fd::AsRawFd;

                // SAFETY: the live file descriptor stays owned by this lease. LOCK_NB never blocks the worker.
                if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                    let error = std::io::Error::last_os_error();

                    return Err(if error.kind() == std::io::ErrorKind::WouldBlock {
                        StorageError::Locked
                    } else {
                        io(error)
                    });
                }
            }
            #[cfg(not(unix))]
            file.try_lock()
                .map_err(|error| StorageError::Io(error.to_string()))?;
            *lease = Some(file);
        }

        Ok(lease)
    }

    fn read(&self, name: &str) -> Result<Option<String>, StorageError> {
        let mut options = OpenOptions::new();

        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = match options.open(self.directory.join(name)) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io(error)),
        };

        private_file(&file)?;
        let mut raw = String::new();

        file.take(MAX_BYTES as u64 + 1)
            .read_to_string(&mut raw)
            .map_err(io)?;

        if raw.len() > MAX_BYTES {
            return Err(StorageError::Invalid(format!(
                "{name} exceeds the library size limit."
            )));
        }

        Ok(Some(raw))
    }

    pub(super) fn load(&self) -> Result<LoadReport, StorageError> {
        let _lease = self.enter()?;

        loaded(
            self.read("library.json")?,
            self.read("recovery.json")?,
            self.read("previous.json")?,
        )
    }

    pub(super) fn write(
        &self,
        envelope: Envelope,
        raw: String,
        expected: u64,
        checkpoint_only: bool,
    ) -> Result<(), StorageError> {
        let _lease = self.enter()?;
        let main = self.read("library.json")?;
        let recovery = self.read("recovery.json")?;
        let mut current = Vec::new();

        for value in [main.as_ref(), recovery.as_ref()].into_iter().flatten() {
            match decode(value) {
                Ok((old, _)) => current.push((old.revision, value)),
                Err(error @ StorageError::Unsupported(_)) => return Err(error),
                Err(_) => {}
            }
        }

        let previous = self.read("previous.json")?;

        if current.is_empty()
            && let Some(value) = previous.as_ref()
        {
            let (old, _) = decode(value)?;

            current.push((old.revision, value));
        }

        current.sort_by_key(|(revision, _)| *revision);
        check_write(
            current.last().map(|(_, raw)| raw.as_str()),
            &envelope,
            expected,
        )?;
        atomic(&self.directory, "recovery.json", raw.as_bytes())?;

        if !checkpoint_only {
            if let Some(main) = main {
                if decode(&main).is_ok() {
                    atomic(&self.directory, "previous.json", main.as_bytes())?;
                } else {
                    atomic(
                        &self.directory,
                        &format!("damaged-{}.json", uuid::Uuid::new_v4()),
                        main.as_bytes(),
                    )?;
                }
            }

            atomic(&self.directory, "library.json", raw.as_bytes())?;
        }

        Ok(())
    }
}

fn private_file(file: &File) -> Result<(), StorageError> {
    let metadata = file.metadata().map_err(io)?;

    if !metadata.is_file() {
        return Err(StorageError::Io(
            "Library data must be a regular file.".into(),
        ));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        // SAFETY: geteuid has no preconditions.
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            return Err(StorageError::Io(
                "Library files must be owned by you and private (mode 0600).".into(),
            ));
        }
    }

    Ok(())
}

fn atomic(directory: &Path, name: &str, bytes: &[u8]) -> Result<(), StorageError> {
    let temporary = directory.join(format!(".{}-{}.tmp", name, uuid::Uuid::new_v4()));

    let result = (|| {
        let mut options = OpenOptions::new();

        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options.open(&temporary).map_err(io)?;

        file.write_all(bytes).map_err(io)?;
        file.sync_all().map_err(io)?;
        std::fs::rename(&temporary, directory.join(name)).map_err(io)?;
        File::open(directory)
            .and_then(|file| file.sync_all())
            .map_err(io)?;

        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }

    result
}
