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

Limits and rules:

- Keys are 1–256 UTF-8 bytes with no NUL. JSON values are at most 256 KiB.
- Pages contain 1–256 keys, sorted by Rust string order, with an exclusive cursor. Concurrent writes can change later pages, so pagination isn't a snapshot.
- There's no implicit default, overwrite on decode failure or automatic migration. A failed read preserves existing data.

**`MemoryBackend::new()`** creates a volatile namespace. Clones share data, and separately constructed backends are isolated.

**`FileBackend::open(directory).await`** creates a native namespace, with file access on the blocking worker pool. Independent handles coordinate through an advisory file lock, and each mutation replaces one map through a synced temporary file and atomic rename. Existing corrupt data fails explicitly. Keys never become filesystem paths.

The file backend is for small stores: 4096 keys, 4 MiB of raw values and a 16 MiB encoded file. It rewrites its JSON map on every change, so use an external database backend for large stores.

- **Permissions:** Unix directories and files must be private, and final symlinks are rejected. The directory and its ancestors must be application-controlled.
- **No encryption:** it does **not** encrypt values. Keychain or encrypted database adapters can implement `Backend`, but this release includes none.
- **Durability:** directory metadata isn't fsynced, so atomic replacement isn't a power-loss guarantee.
- **Cancellation:** cancelling a future may leave a started file operation running, so don't assume it rolls back a write.

Caching is opt-in: `store.with_cache(CacheConfig::new(128, 1024 * 1024)?)`. Cloned handles share a bounded LRU of encoded bytes, including missing keys, and decoded values still don't require `Clone`.

- Budgets count key and value payload bytes, excluding bookkeeping.
- Oversized entries bypass the cache, and key listings are always fresh.
- Writes and removals invalidate across clones, including on failure. An older async read can't repopulate after a newer write.

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

`App::devtools_storage(registry)?` installs the registry as an application service for routed and standalone roots. Headless hosts can install it in `Router::with_services`, and local consumers use `RuntimeClient::request_async` for storage messages.

Native and browser development transports await backend I/O, and ordinary synchronous runtime requests remain supported. Use the normal [DevTools connection setup](devtools.md#connection), because registration doesn't open a development listener.

Chrome's Application → Local/Session storage → `rake://runtime` shows entries as `registered-name/key` with JSON text values. `DOMStorage` get, set, remove and change events are supported, and `Storage.getStorageKey` and the older `Storage.getStorageKeyForFrame` identify the runtime frame.

Reads and edits use the typed facade and its cache. Invalid JSON or types, unknown names and read-only writes fail without replacing data. These edits are persistent application writes, not temporary Element overrides. App views keep their normal refresh behavior, and DevTools doesn't force GPUI rerenders.

Catalog polling reads change revisions only. Cloned store writes and explicit invalidation trigger bounded value refreshes of watched areas, and independent writers still need explicit invalidation. Failed or oversized refreshes stop tracking until a manual table refresh.

Chrome's area view is limited to 256 rows or 384 KiB, and native cursor pages are available through `Query::Storage`. Nothing is silently truncated. Existing transport and event queue bounds also apply.

Persistent edits require retry keys.

- The registry retains at most 256 edit keys or 2 MiB of request payload, including failed edits.
- Matching retries return their saved result, and changed payloads conflict.
- An interrupted or pending edit isn't repeated, because filesystem work may already have committed.
- Reconnecting doesn't reset this ledger, but a new process or registry does.
- Routed hosts also reserve these keys in their application mutation ledger, enforcing its capacity and rejecting key reuse across storage and other commands.

Bulk clear is deliberately unsupported, because registered areas can combine read-only stores and unrelated typed backends.

Without `devtools`, the inspection registry, registration method, store revision
field and revision updates are absent at compile time. There is no DevTools-only
polling, value serialization or inspection snapshot state in production storage.
Normal persistence, optional caching and their normal costs remain.

> Registering encrypted storage would expose **decrypted** data. Keep credentials in separate unregistered stores, or provide an application-owned redacted view.

Caching an encrypted backend would also keep decrypted JSON bytes in memory, so leave caching disabled for such stores. This API doesn't automatically enumerate files, databases or platform secrets.

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
cargo build -p rake-app-cli --offline
cargo build -p rake-app --features devtools --example storage --offline
node scripts/test-storage-cdp.mjs
node scripts/test-storage-cdp.mjs --serve
```

`--serve` prints a Chrome DevTools URL and accepts `light`, `dark`, `read` and `quit` on stdin.

1. Open Application → Local storage → `rake://runtime`.
2. Edit `preferences/editor` using JSON such as `{"theme":"dark","tab_width":8}`.
3. Session storage exposes the read-only `session/example` key.

Chrome has no per-row read-only indicator here. Rejected edits restore the real backend value through a storage event and report the reason in Console. Console also accepts `test.storage({"theme":"light"})` or `test.storage({})` to change or read the same store.

The window's refresh task is demo code, not storage instrumentation. Build with `--no-default-features --features devtools` for a headless-only example.

## Verification

See [test coverage](internal/test-coverage.md) for what's automated.

Stock Chrome 156's Application panel was checked on macOS: origin discovery, typed JSON display, a successful edit read back by the app, an app write updating the table, and invalid or read-only edits restoring the stored row.

Native execution is tested on macOS. WASM compiles, but browser runtime storage and Linux and Windows behavior haven't been verified. Memory storage and the facade are portable, and `FileBackend` is native only.

There's no built-in browser Web Storage, SQLite, Keychain or encryption backend. The byte backend trait is the extension point.
