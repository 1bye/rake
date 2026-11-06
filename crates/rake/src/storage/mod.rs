//! Typed JSON values over application-owned, interchangeable byte stores.
//!
//! Cloning a [`Store`] shares its backend. Values need Serde conversion, not `Clone`,
//! `Send` or `Sync`. Backend futures run on the caller's executor; disk/network
//! implementations must keep blocking work off the application event loop.
//! Opt in to versioned envelopes and migration-on-read with [`Store::with_schema`].
//!
//! ```
//! use rake::storage::{CacheConfig, MemoryBackend, Store};
//! use serde::{Deserialize, Serialize};
//! #[derive(Serialize, Deserialize)]
//! struct Preferences { theme: String }
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! futures::executor::block_on(async {
//!     let store = Store::<Preferences>::new(MemoryBackend::new())
//!         .with_cache(CacheConfig::new(64, 256 * 1024)?);
//!     store.set("editor", &Preferences { theme: "dark".into() }).await?;
//!     assert_eq!(store.clone().get("editor").await?.unwrap().theme, "dark");
//!     Ok::<_, rake::storage::StorageError>(())
//! })?;
//! # Ok(()) }
//! ```

use futures::future::LocalBoxFuture;
use serde::{Serialize, de::DeserializeOwned};

use std::{
    cell::{Cell, RefCell},
    marker::PhantomData,
    rc::Rc,
};

mod cache;
mod schema;

pub use cache::CacheConfig;
pub use schema::Schema;

#[cfg(feature = "devtools")]
pub mod devtools;

mod memory;

pub use memory::MemoryBackend;

#[cfg(not(target_family = "wasm"))]
pub(crate) mod file;

#[cfg(not(target_family = "wasm"))]
pub use file::FileBackend;

/// Largest UTF-8 key accepted by the typed facade, in bytes.
pub const MAX_KEY_BYTES: usize = 256;
/// Largest encoded JSON value accepted by the typed facade, in bytes.
pub const MAX_VALUE_BYTES: usize = 256 * 1024;
/// Largest key page accepted by the typed facade.
pub const MAX_PAGE_SIZE: usize = 256;

/// Storage validation, conversion or backend failure.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// Explicit DevTools registration or page coherence failure.
    #[cfg(feature = "devtools")]
    #[error("storage inspection: {0}")]
    Inspection(String),
    /// Both cache budgets must be nonzero.
    #[error("storage cache requires nonzero entry and byte budgets")]
    InvalidCacheConfig,
    /// A bounded backend cannot admit another key or value.
    #[error("storage backend capacity reached")]
    Capacity,
    /// Keys must contain 1–256 UTF-8 bytes and no NUL.
    #[error("storage key must contain 1–256 bytes and no NUL")]
    InvalidKey,
    /// Page sizes must be 1–256.
    #[error("storage page size must be 1–256")]
    InvalidPage,
    /// An encoded value exceeded the facade's bound.
    #[error("storage value exceeds 256 KiB")]
    ValueTooLarge,
    /// Serialization or typed deserialization failed; existing data is preserved.
    #[error("storage JSON conversion failed: {0}")]
    Codec(#[from] serde_json::Error),
    /// Stored JSON has an unexpected application-defined shape or value.
    #[error("storage data is invalid: {0}")]
    InvalidData(String),
    /// The stored schema belongs to a newer application; the value is preserved.
    #[error("storage schema version {stored} is newer than supported version {current}")]
    NewerVersion {
        /// Version in the stored envelope.
        stored: u64,
        /// Version configured by this application.
        current: u64,
    },
    /// A required adjacent migration was not registered.
    #[error("storage migration from version {from} to the next version is missing")]
    MissingMigration {
        /// Version requiring a migration step.
        from: u64,
    },
    /// A migration was registered twice or does not precede the current version.
    #[error("invalid or duplicate storage migration from version {from} (current: {current})")]
    InvalidMigration {
        /// Invalid or duplicate starting version.
        from: u64,
        /// Configured current version.
        current: u64,
    },
    /// An application migration failed; the original value is preserved.
    #[error("storage migration from version {from} to the next version failed: {source}")]
    Migration {
        /// Starting version of the failed step.
        from: u64,
        /// Original migration failure.
        #[source]
        source: Box<StorageError>,
    },
    /// A backend failed; its original error remains available as the source.
    #[error("storage backend failed: {0}")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// A backend returned keys outside the pagination contract.
    #[error("storage backend returned an invalid key page")]
    InvalidBackendPage,
}

impl StorageError {
    /// Creates a data-validation failure without classifying it as a backend error.
    ///
    /// Migration steps can return this error for invalid old data. The schema wraps
    /// it as the source of [`Self::Migration`], retaining the failing version.
    pub fn invalid_data(message: impl Into<String>) -> Self {
        Self::InvalidData(message.into())
    }
}

impl From<std::io::Error> for StorageError {
    fn from(error: std::io::Error) -> Self {
        Self::Backend(Box::new(error))
    }
}

/// Raw byte storage for one isolated namespace.
///
/// Successful writes replace one value atomically; errors must not partially
/// overwrite it. Removal is idempotent. Key pages are strictly sorted by Rust
/// string order, exclusive of `after`, and contain at most `limit` keys.
/// Implementations own durability, encryption, access control and concurrent-writer
/// coordination. They must not log secret bytes or block an async event loop.
pub trait Backend: 'static {
    /// Reads an owned value, or `None` when absent.
    fn get<'a>(&'a self, key: &'a str)
    -> LocalBoxFuture<'a, Result<Option<Vec<u8>>, StorageError>>;

    /// Atomically replaces a value. Backend errors preserve the previous value.
    fn set<'a>(
        &'a self,
        key: &'a str,
        value: &'a [u8],
    ) -> LocalBoxFuture<'a, Result<(), StorageError>>;

    /// Removes one value; absence is a success.
    fn remove<'a>(&'a self, key: &'a str) -> LocalBoxFuture<'a, Result<(), StorageError>>;

    /// Reads a bounded, sorted key page. Callers pass a valid key cursor and limit.
    fn keys<'a>(
        &'a self,
        after: Option<&'a str>,
        limit: usize,
    ) -> LocalBoxFuture<'a, Result<Vec<String>, StorageError>>;
}

/// A shared backend with typed JSON conversion. Cloning never requires `T: Clone`.
pub struct Store<T> {
    inner: Rc<Inner>,
    value: PhantomData<fn() -> T>,
}

struct Inner {
    backend: Rc<dyn Backend>,
    cache: RefCell<Option<cache::Cache>>,
    epoch: Cell<u64>,
    schema: RefCell<Option<Rc<Schema>>>,
    operations: futures::lock::Mutex<()>,
    writes: Cell<usize>,
    #[cfg(feature = "devtools")]
    revision: Cell<u64>,
}

impl Inner {
    fn invalidate(&self) {
        self.epoch.set(self.epoch.get().wrapping_add(1));
        #[cfg(feature = "devtools")]
        self.revision.set(self.revision.get().wrapping_add(1));

        if let Some(cache) = self.cache.borrow_mut().as_mut() {
            cache.clear();
        }
    }

    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        if self.writes.get() == 0 {
            let cached = self
                .cache
                .borrow_mut()
                .as_mut()
                .and_then(|cache| cache.get(key));

            if let Some(value) = cached {
                return Ok(value);
            }
        }

        let epoch = self.epoch.get();
        let value = self.backend.get(key).await?;

        if let Some(value) = &value {
            validate_value(value)?;
        }

        if self.writes.get() == 0
            && epoch == self.epoch.get()
            && let Some(cache) = self.cache.borrow_mut().as_mut()
        {
            cache.insert(key, value.clone());
        }

        Ok(value)
    }
}

struct Mutation<'a> {
    inner: &'a Inner,
    finished: bool,
}

impl<'a> Mutation<'a> {
    fn new(inner: &'a Inner) -> Self {
        inner.writes.set(inner.writes.get() + 1);
        inner.invalidate();

        Self {
            inner,
            finished: false,
        }
    }

    fn finish(mut self) {
        self.finished = true;
    }
}

impl Drop for Mutation<'_> {
    fn drop(&mut self) {
        self.inner.writes.set(self.inner.writes.get() - 1);
        self.inner.invalidate();

        // A cancelled backend future can still commit on a worker. Bypass caching
        // rather than retaining data read before that unknown commit point.
        if !self.finished {
            *self.inner.cache.borrow_mut() = None;
        }
    }
}

impl<T> Clone for Store<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            value: PhantomData,
        }
    }
}

impl<T> std::fmt::Debug for Store<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store").finish_non_exhaustive()
    }
}

impl<T: Serialize + DeserializeOwned + 'static> Store<T> {
    /// Owns one backend; no I/O or inspection registration occurs here.
    pub fn new(backend: impl Backend) -> Self {
        Self {
            inner: Rc::new(Inner {
                backend: Rc::new(backend),
                cache: RefCell::new(None),
                epoch: Cell::new(0),
                schema: RefCell::new(None),
                operations: futures::lock::Mutex::new(()),
                writes: Cell::new(0),
                #[cfg(feature = "devtools")]
                revision: Cell::new(0),
            }),
            value: PhantomData,
        }
    }

    /// Enables/replaces the encoded-value LRU shared by all cloned handles.
    /// Cache invalidation covers writes through these handles only. Call
    /// [`Self::invalidate`] after independent backend writers; lists are never cached.
    /// A cancelled mutation disables caching because backend work may still commit.
    pub fn with_cache(self, config: CacheConfig) -> Self {
        self.inner.invalidate();
        *self.inner.cache.borrow_mut() = Some(cache::Cache::new(config));

        self
    }

    /// Enables schema envelopes and migration-on-read across cloned handles.
    ///
    /// Configure before starting operations. Reads validate every migration and
    /// the current type before atomic write-back. Versioned get/set/remove calls
    /// serialize across clones; independent stores/backend writers require caller
    /// coordination for the entire read/migrate/write sequence and explicit cache
    /// invalidation. Backend operations already started can outlive cancellation.
    pub fn with_schema(self, schema: Schema) -> Self {
        self.inner.invalidate();
        *self.inner.schema.borrow_mut() = Some(Rc::new(schema));

        self
    }

    /// Drops cached values across clones, including cached absence. External
    /// writers require explicit invalidation after their write has completed.
    pub fn invalidate(&self) {
        self.inner.invalidate();
    }

    /// Loads a value, migrating and writing back when a schema is configured.
    ///
    /// # Errors
    /// Invalid JSON/types, unsupported versions, missing/failing migrations and
    /// oversized results preserve existing data. Backend write-back errors are
    /// returned rather than reporting an upgrade that was not persisted.
    pub async fn get(&self, key: &str) -> Result<Option<T>, StorageError> {
        validate_key(key)?;
        let schema = self.inner.schema.borrow().clone();
        let _operation = if schema.is_some() {
            Some(self.inner.operations.lock().await)
        } else {
            None
        };

        let Some(bytes) = self.inner.get(key).await? else {
            return Ok(None);
        };

        let Some(schema) = schema else {
            return Ok(Some(serde_json::from_slice(&bytes)?));
        };

        let (value, changed) = schema.decode::<T>(&bytes)?;

        if changed {
            let bytes = schema.encode(&value)?;

            validate_value(&bytes)?;
            self.write(key, &bytes).await?;
        }

        Ok(Some(value))
    }

    /// Serializes before changing the backend. Invalid keys/oversized values fail
    /// without writing. Persistence failures preserve the previous backend value.
    pub async fn set(&self, key: &str, value: &T) -> Result<(), StorageError> {
        validate_key(key)?;
        let schema = self.inner.schema.borrow().clone();
        let bytes = match &schema {
            Some(schema) => schema.encode(value)?,
            None => serde_json::to_vec(value)?,
        };

        validate_value(&bytes)?;
        let _operation = if schema.is_some() {
            Some(self.inner.operations.lock().await)
        } else {
            None
        };

        self.write(key, &bytes).await
    }

    async fn write(&self, key: &str, bytes: &[u8]) -> Result<(), StorageError> {
        let mutation = Mutation::new(&self.inner);
        let result = self.inner.backend.set(key, bytes).await;

        mutation.finish();

        result
    }

    /// Deletes a key; deleting a missing key is a success.
    pub async fn remove(&self, key: &str) -> Result<(), StorageError> {
        validate_key(key)?;
        let versioned = self.inner.schema.borrow().is_some();
        let _operation = if versioned {
            Some(self.inner.operations.lock().await)
        } else {
            None
        };

        let mutation = Mutation::new(&self.inner);
        let result = self.inner.backend.remove(key).await;

        mutation.finish();

        result
    }

    /// Lists keys in exclusive cursor order. A short page ends the traversal;
    /// otherwise use the last returned key as the next cursor. Concurrent writers
    /// may change subsequent pages; this is not a database snapshot.
    pub async fn keys(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>, StorageError> {
        if let Some(after) = after {
            validate_key(after)?;
        }

        if !(1..=MAX_PAGE_SIZE).contains(&limit) {
            return Err(StorageError::InvalidPage);
        }

        let keys = self.inner.backend.keys(after, limit).await?;

        if keys.len() > limit
            || keys
                .iter()
                .any(|key| validate_key(key).is_err() || after.is_some_and(|a| key.as_str() <= a))
            || keys.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(StorageError::InvalidBackendPage);
        }

        Ok(keys)
    }
}

fn validate_key(key: &str) -> Result<(), StorageError> {
    if key.is_empty() || key.len() > MAX_KEY_BYTES || key.contains('\0') {
        return Err(StorageError::InvalidKey);
    }

    Ok(())
}

fn validate_value(bytes: &[u8]) -> Result<(), StorageError> {
    if bytes.len() > MAX_VALUE_BYTES {
        return Err(StorageError::ValueTooLarge);
    }

    Ok(())
}
