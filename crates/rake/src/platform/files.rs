//! File selection and export with opaque read handles and bounded reads.

use super::{Error, Platform, Request};

/// A user-selected read handle. Clones refer to the same selection, not cached bytes.
#[derive(Clone)]
pub struct FileHandle {
    pub(super) name: String,
    #[cfg(not(target_family = "wasm"))]
    path: std::path::PathBuf,
    #[cfg(target_family = "wasm")]
    pub(super) file: wasm_bindgen::JsValue,
}

impl std::fmt::Debug for FileHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileHandle")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl FileHandle {
    /// The selected filename, without exposing browser-incompatible path semantics.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Uses an explicit native path. Selection does not read the file.
    #[cfg(not(target_family = "wasm"))]
    pub fn from_path(path: impl Into<std::path::PathBuf>) -> Self {
        let path = path.into();

        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();

        Self { name, path }
    }

    /// Native extension for applications that need filesystem integration.
    #[cfg(not(target_family = "wasm"))]
    pub fn native_path(&self) -> &std::path::Path {
        &self.path
    }

    /// Reads up to the caller's byte limit; native blocking work runs off-thread.
    /// Dropping the future rejects observation but cannot interrupt an in-flight
    /// filesystem call. A native path may have changed since selection.
    ///
    /// # Errors
    /// Returns size, I/O, permission or host access failures.
    pub async fn read(&self, max_bytes: usize) -> Result<Vec<u8>, Error> {
        #[cfg(target_family = "wasm")]
        return super::browser::read_file(&self.file, max_bytes).await;

        #[cfg(not(target_family = "wasm"))]
        {
            let path = self.path.clone();

            blocking::unblock(move || {
                use std::io::Read;
                let mut options = std::fs::OpenOptions::new();

                options.read(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    // A picker result can be replaced with a FIFO before opening.
                    // Never leave a blocking worker waiting forever for its writer.
                    options.custom_flags(libc::O_NONBLOCK);
                }
                let file = options.open(path)?;

                if !file.metadata()?.is_file() {
                    return Err(Error::InvalidInput(
                        "selection must be a regular file".into(),
                    ));
                }

                let mut bytes = Vec::new();

                file.take((max_bytes as u64).saturating_add(1))
                    .read_to_end(&mut bytes)?;

                if bytes.len() > max_bytes {
                    return Err(Error::SizeLimit(max_bytes));
                }

                Ok(bytes)
            })
            .await
        }
    }
}

/// Browser downloads have no filesystem completion acknowledgement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveOutcome {
    /// The native file was written and synced successfully.
    Saved,
    /// A browser download was requested; the user/host may still cancel it.
    DownloadRequested,
    /// The user dismissed the native save dialog.
    Cancelled,
}

impl Platform {
    /// Starts a single-file picker now; await the result in owned work. Browser
    /// selection uses a file input and returns `None` when the user cancels.
    pub fn pick_file(&self, cx: &gpui::App) -> Request<Option<FileHandle>> {
        #[cfg(target_family = "wasm")]
        {
            let _ = cx;

            super::browser::pick_file()
        }

        #[cfg(not(target_family = "wasm"))]
        {
            let selection = cx.prompt_for_paths(gpui::PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: None,
            });

            Box::pin(async move {
                let paths = selection
                    .await
                    .map_err(|_| Error::Unavailable {
                        operation: "file picker",
                        message: "host closed the selection channel".into(),
                    })?
                    .map_err(|error| Error::Unavailable {
                        operation: "file picker",
                        message: error.to_string(),
                    })?;

                Ok(paths
                    .and_then(|paths| paths.into_iter().next())
                    .map(FileHandle::from_path))
            })
        }
    }

    /// Starts a native save dialog or browser download immediately. Uses only a
    /// filename, not a path. Native writing happens after selection, off-thread.
    pub fn export_file(
        &self,
        filename: &str,
        bytes: Vec<u8>,
        cx: &gpui::App,
    ) -> Request<SaveOutcome> {
        if filename.is_empty()
            || matches!(filename, "." | "..")
            || filename
                .chars()
                .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
        {
            return Box::pin(async {
                Err(Error::InvalidInput(
                    "export requires a plain filename".into(),
                ))
            });
        }

        #[cfg(target_family = "wasm")]
        {
            let _ = cx;
            let result = super::browser::export_file(filename, &bytes);

            Box::pin(async move { result.map(|()| SaveOutcome::DownloadRequested) })
        }

        #[cfg(not(target_family = "wasm"))]
        {
            let root = std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(std::path::PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);

            let selection = cx.prompt_for_new_path(&root, Some(filename));

            Box::pin(async move {
                let path = selection
                    .await
                    .map_err(|_| Error::Unavailable {
                        operation: "save picker",
                        message: "host closed the selection channel".into(),
                    })?
                    .map_err(|error| Error::Unavailable {
                        operation: "save picker",
                        message: error.to_string(),
                    })?;

                let Some(path) = path else {
                    return Ok(SaveOutcome::Cancelled);
                };

                blocking::unblock(move || {
                    use std::io::Write;
                    let mut file =
                        tempfile::NamedTempFile::new_in(path.parent().ok_or_else(|| {
                            Error::InvalidInput("save path has no parent".into())
                        })?)?;

                    file.write_all(&bytes)?;
                    file.as_file().sync_all()?;
                    file.persist(path).map_err(|error| Error::Io(error.error))?;

                    Ok(SaveOutcome::Saved)
                })
                .await
            })
        }
    }
}
