//! Optional bounded LRU of encoded values, including absent keys.

use super::StorageError;
use std::collections::{BTreeMap, VecDeque};

/// Cache bounds count UTF-8 keys plus encoded value bytes, not allocator overhead.
#[derive(Clone, Copy, Debug)]
pub struct CacheConfig {
    entries: usize,
    bytes: usize,
}

impl CacheConfig {
    /// Requires nonzero entry and byte budgets. Oversized individual entries are
    /// read normally but never cached. Decoding still happens on every typed read.
    pub fn new(max_entries: usize, max_bytes: usize) -> Result<Self, StorageError> {
        if max_entries == 0 || max_bytes == 0 {
            return Err(StorageError::InvalidCacheConfig);
        }
        Ok(Self {
            entries: max_entries,
            bytes: max_bytes,
        })
    }
}

pub(super) struct Cache {
    config: CacheConfig,
    values: BTreeMap<String, Option<Vec<u8>>>,
    order: VecDeque<String>,
    bytes: usize,
}

impl Cache {
    pub(super) fn new(config: CacheConfig) -> Self {
        Self {
            config,
            values: BTreeMap::new(),
            order: VecDeque::new(),
            bytes: 0,
        }
    }
    pub(super) fn clear(&mut self) {
        self.values.clear();
        self.order.clear();
        self.bytes = 0;
    }
    pub(super) fn get(&mut self, key: &str) -> Option<Option<Vec<u8>>> {
        let value = self.values.get(key)?.clone();
        self.order.retain(|existing| existing != key);
        self.order.push_back(key.to_owned());
        Some(value)
    }
    pub(super) fn insert(&mut self, key: &str, value: Option<Vec<u8>>) {
        let cost = key.len() + value.as_ref().map_or(0, Vec::len);
        if cost > self.config.bytes {
            return;
        }
        self.remove(key);
        while self.values.len() >= self.config.entries || self.bytes > self.config.bytes - cost {
            let Some(oldest) = self.order.front().cloned() else {
                break;
            };
            self.remove(&oldest);
        }
        self.bytes += cost;
        self.order.push_back(key.to_owned());
        self.values.insert(key.to_owned(), value);
    }
    fn remove(&mut self, key: &str) {
        if let Some(value) = self.values.remove(key) {
            self.bytes -= key.len() + value.as_ref().map_or(0, Vec::len);
        }
        self.order.retain(|existing| existing != key);
    }
}
