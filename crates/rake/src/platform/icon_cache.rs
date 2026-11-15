//! Byte-bounded, least-recently-used storage for native icon rasters.

#[cfg(any(target_os = "macos", test))]
use std::collections::HashMap;
#[cfg(any(target_os = "macos", test))]
use std::sync::Arc;

/// Default raster budget, excluding metadata and renderer-owned resources.
pub const DEFAULT_BUDGET: usize = 4 * 1024 * 1024;

/// The visual identity of an icon, independent of its requested raster size.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Identity {
    Folder,
    FileType(String),
    #[cfg(any(target_os = "macos", test))]
    Bundle(String),
    #[cfg(any(target_os = "macos", target_os = "windows", test))]
    Custom(std::path::PathBuf),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Key {
    pub identity: Identity,
    pub pixels: u32,
}

#[cfg(any(target_os = "macos", test))]
struct Entry {
    bytes: Arc<[u8]>,
    used: u64,
}

#[cfg(any(target_os = "macos", test))]
pub struct Cache {
    entries: HashMap<Key, Entry>,
    budget: usize,
    bytes: usize,
    clock: u64,
    hits: u64,
    misses: u64,
}

#[cfg(any(target_os = "macos", test))]
impl Cache {
    pub fn new(budget: usize) -> Self {
        Self {
            entries: HashMap::new(),
            budget,
            bytes: 0,
            clock: 0,
            hits: 0,
            misses: 0,
        }
    }

    #[cfg(test)]
    pub fn stats(&self) -> (usize, usize, u64, u64) {
        (self.entries.len(), self.bytes, self.hits, self.misses)
    }

    pub fn get(&mut self, key: &Key) -> Option<Arc<[u8]>> {
        self.clock = self.clock.saturating_add(1);

        if let Some(entry) = self.entries.get_mut(key) {
            entry.used = self.clock;
            self.hits += 1;

            return Some(entry.bytes.clone());
        }

        self.misses += 1;

        None
    }

    pub fn insert(&mut self, key: Key, bytes: Arc<[u8]>) {
        if bytes.len() > self.budget {
            return;
        }

        if let Some(previous) = self.entries.remove(&key) {
            self.bytes -= previous.bytes.len();
        }

        while self.bytes + bytes.len() > self.budget {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };

            if let Some(entry) = self.entries.remove(&oldest) {
                self.bytes -= entry.bytes.len();
            }
        }

        self.clock = self.clock.saturating_add(1);
        self.bytes += bytes.len();
        self.entries.insert(
            key,
            Entry {
                bytes,
                used: self.clock,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_thousand_plain_folders_share_one_bitmap() {
        let mut cache = Cache::new(DEFAULT_BUDGET);
        let bitmap: Arc<[u8]> = vec![0; 44 * 44 * 4].into();

        let key = Key {
            identity: Identity::Folder,
            pixels: 44,
        };

        cache.insert(key.clone(), bitmap.clone());

        for _ in 0..10_000 {
            assert!(Arc::ptr_eq(&cache.get(&key).unwrap(), &bitmap));
        }

        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.bytes, 44 * 44 * 4);
        assert_eq!(cache.hits, 10_000);
        assert_eq!(cache.stats(), (1, 44 * 44 * 4, 10_000, 0));
        assert!(cache.bytes <= DEFAULT_BUDGET);
    }

    #[test]
    fn identity_and_display_size_separate_rasters() {
        let mut cache = Cache::new(DEFAULT_BUDGET);

        let identities = [
            Identity::Folder,
            Identity::FileType("text/plain".into()),
            Identity::Bundle("example.application".into()),
            Identity::Custom("custom-folder".into()),
        ];

        for identity in identities {
            for pixels in [22, 44] {
                let key = Key {
                    identity: identity.clone(),
                    pixels,
                };

                assert!(cache.get(&key).is_none());
                cache.insert(
                    key.clone(),
                    vec![0; pixels as usize * pixels as usize * 4].into(),
                );
                assert!(cache.get(&key).is_some());
            }
        }

        assert_eq!(cache.entries.len(), 8);
    }

    #[test]
    fn eviction_keeps_recent_entries_and_bounds_bytes() {
        let mut cache = Cache::new(8);

        let key = |pixels| Key {
            identity: Identity::Folder,
            pixels,
        };

        cache.insert(key(1), vec![0; 4].into());
        cache.insert(key(2), vec![0; 4].into());
        assert!(cache.get(&key(1)).is_some());
        cache.insert(key(3), vec![0; 4].into());
        assert!(cache.get(&key(2)).is_none());
        assert!(cache.get(&key(1)).is_some());

        for pixels in 4..10_000 {
            cache.insert(key(pixels), vec![0; 4].into());
            assert!(cache.bytes <= 8);
        }

        cache.insert(key(10_001), vec![0; 9].into());
        assert!(cache.get(&key(10_001)).is_none());
        assert_eq!(cache.entries.len(), 2);
    }
}
