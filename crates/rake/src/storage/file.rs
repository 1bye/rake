//! Small native stores with serialized, atomic whole-file replacement.

use super::{Backend, MAX_PAGE_SIZE, StorageError, validate_key, validate_value};
use futures::future::LocalBoxFuture;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const MAX_KEYS: usize = 4096;
const MAX_TOTAL_BYTES: usize = 4 * 1024 * 1024;
const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;
type Values = BTreeMap<String, Vec<u8>>;

/// Native, unencrypted storage for small preference/document namespaces.
///
/// Each operation uses the blocking worker pool and an advisory lock shared by
/// independently opened instances. Writes sync a private temporary file before
/// atomic rename. The directory must be application-owned and trusted; Unix
/// directories/files must exclude group/other permissions and symlinks are refused.
/// Values are JSON byte arrays, bounded to 4096 keys / 4 MiB total raw value bytes /
/// 16 MiB encoded file. This backend rewrites the whole map on mutation.
///
/// Cancelling an awaited operation does not cancel an already-running filesystem
/// operation. Rename is the commit point; directory metadata is not fsynced, so
/// this is not a power-loss durability guarantee or a multi-key transaction.
#[derive(Clone, Debug)]
pub struct FileBackend {
    directory: PathBuf,
}

impl FileBackend {
    /// Creates/opens a private directory on a worker. Existing corrupt data is
    /// reported by reads/mutations and is never replaced with an empty map.
    pub async fn open(directory: impl AsRef<Path>) -> Result<Self, StorageError> {
        let directory = directory.as_ref().to_path_buf();
        blocking::unblock(move || {
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(&directory)?;
            check_directory(&directory)?;
            Ok::<_, StorageError>(Self {
                directory: fs::canonicalize(directory)?,
            })
        })
        .await
    }

    async fn access<R: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Values) -> Result<(R, bool), StorageError> + Send + 'static,
    ) -> Result<R, StorageError> {
        let directory = self.directory.clone();
        blocking::unblock(move || {
            check_directory(&directory)?;
            let lock = private_options()
                .read(true)
                .write(true)
                .create(true)
                .open(directory.join("store.lock"))?;
            check_file(&lock)?;
            lock.lock()?;
            let path = directory.join("store.json");
            let mut values = match private_options().read(true).open(&path) {
                Ok(file) => {
                    check_file(&file)?;
                    let mut bytes = Vec::new();
                    file.take((MAX_FILE_BYTES + 1) as u64)
                        .read_to_end(&mut bytes)?;
                    if bytes.len() > MAX_FILE_BYTES {
                        return Err(StorageError::Capacity);
                    }
                    serde_json::from_slice(&bytes)?
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => Values::new(),
                Err(error) => return Err(error.into()),
            };
            check_values(&values)?;
            let (result, changed) = work(&mut values)?;
            if changed {
                check_values(&values)?;
                let bytes = serde_json::to_vec(&values)?;
                if bytes.len() > MAX_FILE_BYTES {
                    return Err(StorageError::Capacity);
                }
                let temporary =
                    Temporary(directory.join(format!(".store-{}.tmp", uuid::Uuid::new_v4())));
                let mut file = private_options()
                    .write(true)
                    .create_new(true)
                    .open(&temporary.0)?;
                file.write_all(&bytes)?;
                file.sync_all()?;
                drop(file);
                fs::rename(&temporary.0, &path)?;
            }
            Ok(result)
        })
        .await
    }
}

impl Backend for FileBackend {
    fn get<'a>(
        &'a self,
        key: &'a str,
    ) -> LocalBoxFuture<'a, Result<Option<Vec<u8>>, StorageError>> {
        Box::pin(async move {
            validate_key(key)?;
            let key = key.to_owned();
            self.access(move |values| Ok((values.get(&key).cloned(), false)))
                .await
        })
    }
    fn set<'a>(
        &'a self,
        key: &'a str,
        value: &'a [u8],
    ) -> LocalBoxFuture<'a, Result<(), StorageError>> {
        Box::pin(async move {
            validate_key(key)?;
            validate_value(value)?;
            let key = key.to_owned();
            let value = value.to_vec();
            self.access(move |values| {
                values.insert(key, value);
                Ok(((), true))
            })
            .await
        })
    }
    fn remove<'a>(&'a self, key: &'a str) -> LocalBoxFuture<'a, Result<(), StorageError>> {
        Box::pin(async move {
            validate_key(key)?;
            let key = key.to_owned();
            self.access(move |values| Ok(((), values.remove(&key).is_some())))
                .await
        })
    }
    fn keys<'a>(
        &'a self,
        after: Option<&'a str>,
        limit: usize,
    ) -> LocalBoxFuture<'a, Result<Vec<String>, StorageError>> {
        Box::pin(async move {
            if let Some(after) = after {
                validate_key(after)?;
            }
            if !(1..=MAX_PAGE_SIZE).contains(&limit) {
                return Err(StorageError::InvalidPage);
            }
            let after = after.map(str::to_owned);
            self.access(move |values| {
                Ok((
                    values
                        .keys()
                        .filter(|key| after.as_deref().is_none_or(|a| key.as_str() > a))
                        .take(limit)
                        .cloned()
                        .collect(),
                    false,
                ))
            })
            .await
        })
    }
}

fn check_values(values: &Values) -> Result<(), StorageError> {
    if values.len() > MAX_KEYS || values.values().map(Vec::len).sum::<usize>() > MAX_TOTAL_BYTES {
        return Err(StorageError::Capacity);
    }
    for (key, value) in values {
        validate_key(key)?;
        validate_value(value)?;
    }
    Ok(())
}

fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    options
}

fn check_file(file: &File) -> io::Result<()> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "storage path must be a regular file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "storage file must be private",
            ));
        }
    }
    Ok(())
}

fn check_directory(directory: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(directory)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "storage directory must be a real directory",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "storage directory must be private",
            ));
        }
    }
    Ok(())
}

struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
