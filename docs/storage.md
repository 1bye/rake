# Typed storage

Implementation sequence (one commit per step):

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
payload bytes, excluding allocator/map/recency bookkeeping. Oversized entries bypass the cache and
key listings are always fresh. Writes/removals invalidate across clones, including
failure paths; an older async read cannot repopulate after a newer write.

Independent writers require `store.invalidate()` after their write completes;
there is no file watcher or cross-process cache coherence. Cancelling a mutation
disables caching across its handles because backend work could still commit later.
Re-enable only after pending backend work is known to have finished. The default
store has no cache and always reads its backend.

With `rake/devtools`, `storage::devtools::Storage` explicitly registers typed
handles using `read_only(name, Area::Local, store)` or `read_write(...)`.
Use `Area::Session` for data intended to last only for the session; this is
presentation metadata, not a change to backend persistence. Registration snapshots
share backend/cache handles. Unregistered stores are never inspected.

`App::devtools_storage(registry)?` installs the registry as an application service
for routed and standalone roots. Headless hosts can install the same registry in
`Router::with_services`; local consumers use `RuntimeClient::request_async` for
storage messages. Native and browser development transports await backend I/O.
Ordinary synchronous runtime requests remain supported.
Use the normal [DevTools connection setup](devtools.md#connection); registration
itself does not open a development listener.

Chrome's Application → Local/Session storage → `rake://runtime` projects entries
as `registered-name/key` with JSON text values. `DOMStorage` get/set/remove and
change events are supported; `Storage.getStorageKey` and the older
`Storage.getStorageKeyForFrame` identify the runtime
frame. Reads/edits use the typed facade and its cache. Invalid JSON/types, unknown
names and read-only writes fail without replacing data. These edits are persistent
application writes, not temporary Element overrides. App views still own their
normal refresh/notification behavior; DevTools does not force GPUI rerenders.

Catalog polling reads change revisions only. Cloned store writes and explicit
invalidation trigger bounded value refreshes of watched areas. Independent writers
still require explicit invalidation. Failed/oversized refreshes stop tracking until
manual table refresh. Chrome's area projection is limited to 256 rows / 384 KiB;
native cursor pages are separately available through `Query::Storage`. There is
no silent truncation. Existing transport/event queue bounds also apply.

Persistent edits require retry keys. The registry retains at most 256 edit keys /
2 MiB of request payload, including failed edits. Matching retries return their
saved result; changed payloads conflict. An interrupted/pending edit is not repeated
because filesystem work may already have committed. Reconnect does not reset this
ledger; a new process/registry does. Routed hosts also reserve these keys in their
ordinary application mutation ledger, enforcing its capacity and rejecting key
reuse across storage and other commands. Bulk clear is deliberately unsupported, since
registered areas can combine read-only stores and unrelated typed backends.

Without `devtools`, the inspection registry, registration method, store revision
field and revision updates are absent at compile time. There is no DevTools-only
polling, value serialization or inspection snapshot state in production storage.
Normal persistence, optional caching and their normal costs remain.

Registering encrypted storage would expose **decrypted** data. Keep credentials
in separate unregistered stores, or provide an application-owned redacted view.
Caching an encrypted backend would also retain decrypted JSON bytes in application
memory; encryption at rest does not change that. Leave caching disabled for such stores.
This API does not automatically enumerate files, databases or platform secrets.

## Usage

```rust
use rake::storage::{CacheConfig, FileBackend, Store};

#[derive(serde::Serialize, serde::Deserialize)]
struct Preferences {
    theme: String,
    tab_width: u8,
}

// Supply an application-controlled directory; FileBackend creates it privately.
let preferences = Store::<Preferences>::new(FileBackend::open(directory).await?)
    .with_cache(CacheConfig::new(128, 1024 * 1024)?);

preferences.set("editor", &Preferences {
    theme: "dark".into(),
    tab_width: 4,
}).await?;
let loaded = preferences.get("editor").await?;
```

Inspection registration belongs behind an application feature that forwards to
`rake/devtools` (for example, `[features] devtools = ["rake/devtools"]`):

```rust
#[cfg(feature = "devtools")]
let app = {
    use rake::storage::devtools::{Area, Storage};
    let storage = Storage::new()
        .read_write("preferences", Area::Local, preferences.clone())?;
    app.devtools_storage(storage)?
};
```

The runnable Unix storage example includes a cached file-backed preference store,
a read-only session store and a typed `test.storage` application command. Default
GPUI builds show a window with live settings and light/dark theme buttons. The
automated script explicitly uses `--headless` to avoid GPU initialization:

```sh
cargo build -p rake-cli --offline
cargo build -p rake --features devtools --example storage --offline
node scripts/test-storage-cdp.mjs
node scripts/test-storage-cdp.mjs --serve
```

`--serve` prints a Chrome DevTools URL and accepts `light`, `dark`, `read`, and
`quit` on stdin. Open Application → Local storage → `rake://runtime`; edit
`preferences/editor` using JSON such as `{"theme":"dark","tab_width":8}`.
Session storage exposes the read-only `session/example` key. Chrome has no
per-row read-only affordance here: rejected edits restore the actual backend
value through a storage event and report the reason in Console. No browser
security guard is bypassed. Console accepts `test.storage({"theme":"light"})`
or `test.storage({})` to change/read the same typed store.
The window's refresh task is explicit demo code, not storage instrumentation.
Build with `--no-default-features --features devtools` for a headless-only example.

## Verification

The public consumer suites exercise typed round trips without `Clone`, Unicode
keys, malformed/oversized data, real file reopen/concurrent writers/corrupt-file
preservation/capacity/private paths, LRU eviction and absence caching, shared and
external invalidation, delayed reads and cancelled writes. DevTools tests cover
typed/read-only errors, bounded pages, shutdown/stale identities, retry keys shared
with ordinary commands, pending/cancelled edits and shared registries across runtimes.
The standalone host has a separate async storage/teardown test.

The real native/CDP script verifies disk persistence across process restart, typed
validation, application-to-Chrome change events, writable/read-only permissions,
discovery compatibility and recovery of rejected optimistic edits. It never drives
Chrome itself. Stock Chrome 156.0.8078.74's Application panel was additionally
checked on macOS on 2026-10-30:
local/session origin discovery, typed JSON display/preview, a successful preference
edit read back by the application, an application write updating the visible table,
and invalid/read-only edits restoring the actual stored row.

Native execution is tested on macOS. The supported GPUI/WASM configuration compiles;
WASM runtime/browser storage execution and Linux/Windows runtime behavior have not
been verified. Memory storage and the facade are portable; `FileBackend` is native
only. There is no built-in browser Web Storage, SQLite, Keychain or encryption
backend in this implementation. The byte backend trait is the extension point.
