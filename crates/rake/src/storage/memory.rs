//! Shared local, volatile byte storage.

use super::{Backend, MAX_PAGE_SIZE, StorageError, validate_key, validate_value};
use futures::future::LocalBoxFuture;
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

/// Volatile storage. Clones share the same map; dropping the last handle frees it.
/// No persistence, encryption or cross-process sharing is provided.
#[derive(Clone, Debug, Default)]
pub struct MemoryBackend(Rc<RefCell<BTreeMap<String, Vec<u8>>>>);

impl MemoryBackend {
    /// Creates an empty namespace.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Backend for MemoryBackend {
    fn get<'a>(
        &'a self,
        key: &'a str,
    ) -> LocalBoxFuture<'a, Result<Option<Vec<u8>>, StorageError>> {
        Box::pin(async move {
            validate_key(key)?;
            Ok(self.0.borrow().get(key).cloned())
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
            self.0.borrow_mut().insert(key.into(), value.into());
            Ok(())
        })
    }
    fn remove<'a>(&'a self, key: &'a str) -> LocalBoxFuture<'a, Result<(), StorageError>> {
        Box::pin(async move {
            validate_key(key)?;
            self.0.borrow_mut().remove(key);
            Ok(())
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
            Ok(self
                .0
                .borrow()
                .keys()
                .filter(|key| after.is_none_or(|a| key.as_str() > a))
                .take(limit)
                .cloned()
                .collect())
        })
    }
}
