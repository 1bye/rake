# Text Toolkit

A route-free native/Web file utility built with `rake::view!`. Unlike Notes, this
application works on user-selected files rather than a private document database.

```sh
cargo run -p rake-files --locked
```

Import UTF-8 text (up to 128 KiB), inspect its statistics/preview, normalize CRLF/CR
line endings, optionally trim trailing spaces/tabs, then copy or export the result.
The trim preference survives restart. Unicode characters count scalar values;
words are delimited by ASCII whitespace. The preview is capped at 600 characters.

The implementation exercises:

- Shared file import/export, clipboard, preferences and runtime capabilities.
- `runtime.action(cx, future)` for finite user work; no weak references, revision
  watching or custom redraw tasks. Start browser requests inside the click handler.
- View-bound startup preferences and deployment checking through loaders/retry.
- Native background processing and an owned browser module worker using the same
  bounded processing contract.
- A shared typed workspace and `document.statistics` command. GUI and command
  handlers read the same service. Document revisions reject late transform/export
  results; picker cancellation and failed exports preserve existing state.
- App-level theme installation and a shutdown guard shared with update restart.
- Native updates when a packaged policy is supplied, plus Web version checking,
  guarded reload and opt-in production offline assets.

Native exports become clean only after a successful synced write. Browsers cannot
acknowledge filesystem save: **DownloadRequested** keeps an unsaved transformation
dirty until the user explicitly confirms the download or discards it. Copying text
does not imply a save. Source files change only through an explicit export choice.
Selected content is in memory; forced termination can lose an unexported transformation.
The source remains available to import again. Notes demonstrates durable draft recovery.

Build the browser app with matching wasm-bindgen-cli on PATH:

```sh
python3 scripts/build-web.py --example files
python3 scripts/serve-web.py --example files --port 8082
```

Produce a deployment with immutable/offline assets:

```sh
cargo run -p rake-cli --locked -- bundle --target web --package rake-files --debug --output dist/files-release
python3 scripts/serve-web.py --directory dist/files-release/site --port 8082
```

Follow [Web deployment order and retention](../../docs/web-applications.md).
The ordinary development build does not install a service worker. Production
control is disabled; native development inspection/commands are opt-in:

```sh
RAKE_DEV_CONTROL=1 cargo run -p rake-files --locked
rake ctl commands run document.statistics --args null --json
```

The icon, Windows manifest and Web configuration are in Cargo.toml/build.rs. To
enable native feeds/signing/publication, use the [desktop configuration](../desktop/README.md)
with this app's identity and release credentials; no demo endpoint is trusted.

`cargo test -p rake-files --lib` checks workspace revision/export behavior and native
processing. `node scripts/test-file-processing.mjs` checks the browser worker against
the same fixtures. Neither substitutes for real host permission/picker UI checks.
