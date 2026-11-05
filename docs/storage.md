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
- There's no implicit default or overwrite on decode failure. Schema migrations are opt-in, and a failed read preserves existing data.

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

## Schema versions and migrations

Stores without `with_schema` read and write the same bare JSON bytes as before.
Opt in before starting operations:

```rust
use rake::storage::{MemoryBackend, Schema, StorageError, Store};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Preferences {
    theme: String,
    tab_width: u8,
}

fn preferences_schema() -> Result<Schema, StorageError> {
    Schema::new(2)
        .with_migration(0, |mut value| {
            let object = value.as_object_mut().ok_or_else(|| {
                StorageError::Backend(Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData, "expected preferences object",
                )))
            })?;
            if let Some(theme) = object.remove("colour") {
                object.insert("theme".into(), theme);
            }
            Ok(value)
        })?
        .with_migration(1, |mut value| {
            value["tab_width"] = serde_json::json!(4);
            Ok(value)
        })
}
let preferences = Store::<Preferences>::new(MemoryBackend::new())
    .with_schema(preferences_schema()?);
```

Each key stores an envelope such as
`{"$rake_schema":2,"value":{"theme":"dark","tab_width":4}}`.
Unwrapped legacy JSON is version **0**, even if its application fields include
`version` or `format`. The root `$rake_schema` field is reserved for the envelope;
malformed envelopes fail explicitly and are never retried as legacy data.
The 256 KiB value budget includes the envelope. `Schema::new(0)` can wrap legacy
data without a transformation. `set` always writes the configured current version.
It is an explicit replacement operation; load existing data first when downgrade
protection is needed before editing it.

Each registered step transforms JSON from `n` to `n + 1`. Registration rejects
duplicates and steps at or above the current version. Reads reject missing steps
and data newer than the configured version with inspectable `StorageError`
variants. Supply every step needed for the oldest version you support. Keep steps
deterministic and free of I/O or external side effects: retries can run them again.
Validate the old JSON shape and return an error rather than panicking on bad input.

A read runs all steps, decodes the final `T`, serializes its current envelope and
checks its size **before** writing. Failed steps, type conversion, serialization,
size checks and backend writes preserve the original bytes. A successful read
returns only after write-back succeeds. Write-back uses the ordinary mutation
guard: cache epochs and DevTools revisions advance, cached legacy bytes are
cleared, and subsequent reads cache the current envelope. DevTools displays/edits
`T` without the envelope. A page that triggers migration reports a coherence
error; refresh it to read the upgraded values.

Versioned get/set/remove operations serialize across cloned `Store` handles so a
migration cannot overwrite a concurrent write from another clone. Independently
constructed stores, raw backend writers and other processes must coordinate the
**entire read/migrate/write sequence** themselves and invalidate separate caches.
The file backend lock makes each individual backend operation atomic; it does not
make a pair of get/set calls a transaction. Prefer cloned handles within an app.
Cancelling an in-flight write-back has the same limits as any backend mutation:
started work may still commit, and caching is disabled across clones. Wait for
that work to finish before allowing independent writes or re-enabling the cache.

`Schema::decode::<T>(include_bytes!("fixtures/v0.json"))` is a fixture helper using
the exact migration/typed-decoding path used by `Store`. It returns `(value,
needs_write_back)` without touching a backend; assert the value against the
expected current type. It reports migration and typed conversion failures:

```rust
let (preferences, upgraded) = preferences_schema()?.decode::<Preferences>(
    include_bytes!("fixtures/preferences-v0.json"),
)?;
assert!(upgraded);
assert_eq!(preferences.tab_width, 4);
```

`Schema::encode` and `decode` also support custom transactional repositories.
They perform no I/O and impose no size bound; the repository must validate its
domain, enforce its limits and commit safely. `examples/notes` uses these codecs
around its revisioned library snapshot, retaining its 8 MiB bound, native lease,
browser snapshot comparison, recovery checkpoint and previous-file backup. Both
pre-schema `format: 1` snapshots and the older `format: 0` notes migrate to schema
1. Notes marks an upgrade dirty and writes it through the normal save transaction,
keeping the original saved copy as `previous.json` before replacement. Its checked-in
`tests/fixtures/library-v0.json` is the pre-schema format and is tested through
upgrade, save and reopen.

## DevTools

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

Migration tests cover 0→1→2 chains, legacy and versioned inputs, fixture decoding,
failed steps/type checks/serialization/write-back, oversized upgrades, newer
versions, concurrent cloned handles, independent cache invalidation and cache reuse
on memory and file backends. Notes tests cover the pre-schema fixture, backup
preservation, save/reopen and newer-schema rejection.

Stock Chrome 156's Application panel was checked on macOS: origin discovery, typed JSON display, a successful edit read back by the app, an app write updating the table, and invalid or read-only edits restoring the stored row.

Native execution is tested on macOS. Headless memory/file storage, schema migrations
and Notes persistence are also tested on Linux. WASM compiles, but browser runtime
storage, Linux GUI behavior and Windows behavior remain unverified. Memory storage
and the facade are portable, and `FileBackend` is native only.

There's no built-in browser Web Storage, SQLite, Keychain or encryption backend. The byte backend trait is the extension point.
