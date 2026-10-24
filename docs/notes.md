# Fieldnotes reference application

Fieldnotes is a working local-first notes app, with an application-owned shell,
search across titles and bodies, pinned notes, recoverable Archive, document
selection, Unicode text editing, wrapping, IME input, bounded undo/redo and
Markdown clipboard export. It uses Rake routes, managed activities, typed service
composition, commands, reload checkpoints, dirty-entry guards and inspection.

```sh
cargo run -p rake-notes
# Development runtime and graphical inspector
cargo run -p rake-cli -- dev --package rake-notes
# Application bundle, ad-hoc signed by default on macOS
cargo run -p rake-cli -- bundle --package rake-notes
# Browser artifacts (matching wasm-bindgen-cli required)
cargo run -p rake-cli -- build --target web --package rake-notes
```

The window opens at 1220 × 820, with a 960 × 640 minimum. Cmd/Ctrl N creates a
note, F focuses search and S saves now. The title and body are independent text
fields; clipboard shortcuts, selection and undo operate in the focused field.
Notes are plain text with Markdown-friendly content; this is not a rich-text or
collaborative editor.

## Save and recovery contract

Edits update the shared notebook immediately. A managed Live activity writes a
recovery checkpoint, then saves the main snapshot after 700 ms without changes.
Continuous typing causes a main save after two seconds. A normal guarded quit
flushes the latest revision, including edits made while a previous write runs.
Failed saves retain dirty data and offer retry, close without saving, or cancel.
“Saved on this device” appears only after the main write is acknowledged. A
recovery checkpoint alone does not claim a normal save.

Abrupt termination can lose edits that have not reached a checkpoint. A
checkpoint acknowledged before termination survives restart. Loading chooses
the newest valid saved/recovery snapshot and reports recovery; if both are
damaged, it can use the previous saved copy. Unsupported future formats and
unreadable existing libraries show an error instead of seeding a new library.
Format 0 migrates titles/plain text to format 1, preserving the original copy.

Native storage uses a dedicated directory, an exclusive process lease, serialized
worker I/O, private regular files, file fsync, atomic rename and directory fsync.
The main files are `library.json`, `recovery.json`, `previous.json` and
`library.lock`. Repair preserves a damaged main snapshot as `damaged-*.json`.
Default locations are:

- macOS: `~/Library/Application Support/dev.rake.fieldnotes`
- Linux: `$XDG_DATA_HOME/fieldnotes`, falling back to `~/.local/share/fieldnotes`

`RAKE_NOTES_DATA_DIR` selects another dedicated native library. Back up the entire
directory after Save now or a graceful quit. These snapshots are recovery aids,
not a substitute for an independent backup. Symlinks and non-private files are
rejected. The native implementation has been exercised on macOS, not Linux or
Windows.

Web storage is origin-local `localStorage`, serialized across windows with Web
Locks and optimistic revisions. Native and Web writes use the same full document
validator. Web compare-and-write also checks every raw snapshot under the lock,
so validation cannot race another writer. Use localhost or HTTPS and a WebGPU-capable
browser. A stale writer is blocked rather than silently replacing another
window's changes; restart that window to load the latest library. Quota/access
failures stay visible. Clearing site data, private browsing policies or browser
eviction can remove notes; keep exports/backups of important content. There is
no account or sync server.

The Web build embeds licensed Noto Sans JP and Noto Color Emoji fallbacks
(15.3 MB uncompressed font data) because GPUI's canvas renderer cannot use host
system fonts. Japanese text and composed color emoji are exercised in Chrome;
the bundled fonts do not cover every Unicode script. Browser artifacts include
`font-licenses.txt`. Native builds use host fonts.

Libraries are bounded to 1000 documents, 512 KiB per body, 1024 bytes per title
and 8 MiB serialized. Archive retains content and counts toward these limits.
Undo is per selected document session, up to 100 edits/2 MiB, and is not restored
after restart. Search and collection filters are transient; selected note and
content are durable. Reload restores compatible unsaved notebook state and checks
the disk revision before writing. Saved libraries reload from storage. Unsaved
content exceeding Rake's checkpoint limit blocks reload; use Save now and retry.

## Shared commands and inspection

Enable the development endpoint with `RAKE_DEV_CONTROL=1` (or use `rake dev`).
All command arguments/results have generated schemas. Examples:

```sh
rake ctl commands list
rake ctl commands run notes.list --args null
rake ctl commands run notes.create --args '{"title":"An idea","body":"Start small."}'
rake ctl commands run notes.edit --args '{"id":4,"expected_version":1,"title":null,"body":"A better thought."}'
rake ctl commands run notes.save --args '{}'
rake ctl quit
```

`notes.edit` requires the observed version and rejects stale changes. The GUI's
per-keystroke edits call the same typed notebook operation synchronously; create
and Save now dispatch through the shared operation registry. The service is
resolved from `CommandContext.services`. Lists expose summaries, not note bodies.
The Elements panel exposes `notes.workspace`, `notes.title`, `notes.body`,
`notes.search`, `notes.note.<id>`, save and recovery boundaries. Note contents are
not added to diagnostics or inspection metadata.

Verification: portable persistence tests exercise actual files/leases/workers;
`scripts/test-notes.py` exercises a real macOS app and CLI through SIGKILL recovery
and graceful quit. `scripts/test-notes-storage.mjs` executes the actual browser
adapter functions against an in-memory storage/lock host, covering conflicts,
quota failures and recovery. It does not establish live browser/GPU behavior.
The separate [real-browser test procedure](notes-browser-testing.md) records
Chrome input, long-document scrolling, background autosave, stale tabs,
malformed recovery and an actual renderer crash.
