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
