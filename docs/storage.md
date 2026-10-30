# Typed storage

Implementation plan (one commit per step):

1. Typed Serde facade, byte backend contract and public consumer tests.
2. Interchangeable memory and native file backends, with atomic replacement.
3. Optional bounded caching shared by cloned handles; explicit invalidation.
4. Feature-gated, explicitly registered DevTools access through Chrome storage.
5. Example, documentation and supported native/WASM verification.

Storage is a production API independent of inspection. Values implement
`Serialize + DeserializeOwned`; neither cloning values nor thread-safe UI state
is required. A backend owns one namespace and implements async get, set, remove,
and cursor-paged keys. It owns persistence, encryption and access control.

Keys contain 1–256 UTF-8 bytes and no NUL. JSON values are at most 256 KiB.
Pages contain 1–256 keys, sorted by Rust string order with an exclusive cursor.
Concurrent writes can change later pages; pagination is not a snapshot.
There is no implicit default value, overwrite on decode failure, or automatic
migration. A failed read preserves existing data.

`MemoryBackend::new()` creates a volatile namespace. Backend clones share data;
separately constructed backends are isolated. `FileBackend::open(directory).await`
creates a native namespace, running file access on the blocking worker pool.
Independent handles coordinate through an advisory file lock; each mutation
replaces one map through a synced temporary file and atomic rename. Existing
corrupt data fails explicitly. Keys never become filesystem paths.

The file backend is for small stores: 4096 keys, 4 MiB total raw values, 16 MiB
encoded file. It rewrites its JSON byte-array map on every change. Use an external
database backend for large stores. Unix directories/files must be private; final
symlinks are rejected. Its directory and ancestors must be application-controlled.
It does **not** encrypt values. Platform Keychain/encrypted database adapters can
implement `Backend`; this release does not supply a cryptographic implementation.
Directory metadata is not fsynced, so atomic replacement is not a power-loss
durability guarantee. Cancelling a future may leave an already-started file
operation running; do not assume cancellation rolls back a write.

Caching is opt-in: `store.with_cache(CacheConfig::new(128, 1024 * 1024)?)`.
Cloned store handles share a bounded LRU of encoded bytes (including missing
keys); decoded values still do not require `Clone`. Budgets count key and value
bytes, excluding allocator/map overhead. Oversized entries bypass the cache and
key listings are always fresh. Writes/removals invalidate across clones, including
failure paths; an older async read cannot repopulate after a newer write.

Independent writers require `store.invalidate()` after their write completes;
there is no file watcher or cross-process cache coherence. Cancelling a mutation
disables caching across its handles because backend work could still commit later.
Re-enable only after pending backend work is known to have finished. The default
store has no cache and always reads its backend.
