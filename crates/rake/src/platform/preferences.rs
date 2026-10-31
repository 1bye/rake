//! Small namespaced preferences, separate from transactional document storage.

use super::Error;

/// A cloned handle to one preference namespace. Individual replacements are atomic;
/// read/modify/write is last-writer-wins. Use a document store for conflict detection.
#[derive(Clone, Debug)]
pub struct Preferences {
    #[cfg(not(target_family = "wasm"))]
    root: std::path::PathBuf,
    #[cfg(target_family = "wasm")]
    namespace: String,
}

pub(super) fn name(value: &str) -> Result<(), Error> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        || matches!(value, "." | "..")
    {
        return Err(Error::InvalidInput(
            "names must be 1–128 ASCII letters, digits, dots, hyphens or underscores".into(),
        ));
    }

    Ok(())
}

impl Preferences {
    pub(super) fn open(namespace: &str) -> Result<Self, Error> {
        name(namespace)?;

        #[cfg(target_family = "wasm")]
        return Ok(Self {
            namespace: namespace.into(),
        });

        #[cfg(not(target_family = "wasm"))]
        {
            let root = application_data()?;

            Ok(Self {
                root: root.join(namespace),
            })
        }
    }

    /// Uses an explicit native root, useful for portable installs and isolated tests.
    /// No directory is created until the first write.
    #[cfg(not(target_family = "wasm"))]
    pub fn at(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Reads an optional UTF-8 preference, off the native UI thread.
    ///
    /// # Errors
    /// Rejects invalid keys, I/O failures, invalid UTF-8 and browser access denial.
    pub async fn get(&self, key: &str) -> Result<Option<String>, Error> {
        name(key)?;

        #[cfg(target_family = "wasm")]
        return super::browser::preference_get(&self.namespace, key);

        #[cfg(not(target_family = "wasm"))]
        {
            let path = self.root.join(key);

            blocking::unblock(move || match std::fs::read_to_string(path) {
                Ok(value) => Ok(Some(value)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error.into()),
            })
            .await
        }
    }

    /// Atomically replaces one UTF-8 value. Native writes sync a private temporary
    /// file before renaming it; browser localStorage replacement is atomic per key.
    /// Browser storage is origin-local and can be cleared/evicted by the host.
    ///
    /// # Errors
    /// Rejects invalid keys, I/O failures, quota exhaustion and access denial.
    pub async fn set(&self, key: &str, value: &str) -> Result<(), Error> {
        name(key)?;

        #[cfg(target_family = "wasm")]
        return super::browser::preference_set(&self.namespace, key, Some(value));

        #[cfg(not(target_family = "wasm"))]
        {
            let root = self.root.clone();
            let path = root.join(key);
            let value = value.to_owned();

            blocking::unblock(move || {
                use std::io::Write;
                std::fs::create_dir_all(&root)?;
                let mut temporary = tempfile::NamedTempFile::new_in(&root)?;

                temporary.write_all(value.as_bytes())?;
                temporary.as_file().sync_all()?;
                temporary
                    .persist(path)
                    .map_err(|error| Error::Io(error.error))?;

                Ok(())
            })
            .await
        }
    }

    /// Removes one preference; removing a missing value succeeds.
    ///
    /// # Errors
    /// Rejects invalid keys and filesystem/browser access failures.
    pub async fn remove(&self, key: &str) -> Result<(), Error> {
        name(key)?;

        #[cfg(target_family = "wasm")]
        return super::browser::preference_set(&self.namespace, key, None);

        #[cfg(not(target_family = "wasm"))]
        {
            let path = self.root.join(key);

            blocking::unblock(move || match std::fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.into()),
            })
            .await
        }
    }
}

#[cfg(not(target_family = "wasm"))]
fn application_data() -> Result<std::path::PathBuf, Error> {
    #[cfg(target_os = "windows")]
    let root = std::env::var_os("APPDATA").map(std::path::PathBuf::from);

    #[cfg(target_os = "macos")]
    let root = std::env::var_os("HOME")
        .map(|root| std::path::PathBuf::from(root).join("Library/Application Support"));

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME").map(|root| std::path::PathBuf::from(root).join(".config"))
        });

    root.filter(|root| root.is_absolute())
        .ok_or_else(|| Error::Unavailable {
            operation: "preferences",
            message: "no absolute application-data directory is configured".into(),
        })
}
