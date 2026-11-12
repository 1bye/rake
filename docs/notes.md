# Fieldnotes reference application

Fieldnotes is a working local-first notes app, with an application-owned shell,
search across titles and bodies, pinned notes, recoverable Archive, document
selection, Unicode text editing, wrapping, IME input, bounded undo/redo and
Markdown clipboard export. It uses Rake routes, managed activities, typed service
composition, commands, reload checkpoints, dirty-entry guards and inspection.

```sh
cargo run -p rake-notes
# Web development with embedded Chrome DevTools
cargo run -p rake-app-cli -- dev --target web --package rake-notes --cdp
# Application bundle, ad-hoc signed by default on macOS
cargo run -p rake-app-cli -- bundle --package rake-notes
# Browser artifacts (matching wasm-bindgen-cli required)
cargo run -p rake-app-cli -- build --target web --package rake-notes
```

The window opens at 1220 × 820, with a 960 × 640 minimum. Cmd/Ctrl N creates a note, F focuses search and S saves now. The title and body are independent text fields, and clipboard shortcuts, selection and undo work in the focused field.

Notes are plain text with Markdown-friendly content. This isn't a rich-text or collaborative editor.

## Save and recovery contract

Edits update the shared notebook immediately. A managed Live activity writes a recovery checkpoint, then saves the main snapshot after 700 ms without changes. Continuous typing triggers a main save after two seconds.

- A normal guarded quit flushes the latest revision, including edits made during a previous write.
- Failed saves keep dirty data and offer retry, close without saving, or cancel.
- “Saved on this device” appears only after the main write is acknowledged. A recovery checkpoint alone doesn't claim a normal save.

Abrupt termination can lose edits that haven't reached a checkpoint, while a checkpoint acknowledged before termination survives restart.

- Loading chooses the newest valid saved or recovery snapshot and reports recovery. If both are damaged, it can use the previous saved copy.
- Unsupported future formats and unreadable libraries show an error instead of seeding a new library.
- Format 0 migrates titles and plain text to format 1, preserving the original copy.

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

Web storage is origin-local `localStorage`, serialized across windows with Web Locks and optimistic revisions. Native and Web writes use the same document validator, and Web compare-and-write also checks every raw snapshot under the lock. Use localhost or HTTPS and a WebGPU-capable browser.

- A stale writer is blocked rather than silently replacing another window's changes. Restart that window to load the latest library.
- Quota and access failures stay visible.
- Clearing site data, private browsing policies or browser eviction can remove notes, so keep exports of important content.

There's no account or sync server.

The Web build embeds licensed Noto Sans JP and Noto Color Emoji fallbacks
(15.3 MB uncompressed font data) because GPUI's canvas renderer cannot use host
system fonts. Japanese text and composed color emoji are exercised in Chrome;
the bundled fonts do not cover every Unicode script. Browser artifacts include
`font-licenses.txt`. Native builds use host fonts.

Limits: 1000 documents, 512 KiB per body, 1024 bytes per title and 8 MiB serialized. Archived notes keep their content and count toward these limits.

- Undo is per selected document session, up to 100 edits or 2 MiB, and isn't restored after restart.
- Search and collection filters are transient. The selected note and content are durable.
- Reload restores compatible unsaved notebook state and checks the disk revision before writing. Saved libraries reload from storage.
- Unsaved content over Rake's checkpoint limit blocks reload, so use Save now and retry.

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

`notes.edit` requires the observed version and rejects stale changes. The GUI's per-keystroke edits call the same typed notebook operation synchronously, and create and Save now dispatch through the shared operation registry. The service is resolved from `CommandContext.services`.

Lists expose summaries, not note bodies. The shell uses `rake::view!` to capture
`notes.workspace`, its sidebar and collection navigation, keyed note rows, and the
document header, article and editor mounts. Shared `Input` and `Textarea` controls
retain main's keyboard handling, accessibility and byte limits. Rendered labels,
note titles and previews can appear in devtools captures; full editor contents
are excluded from inspection text and diagnostics.

Run `python3 scripts/test-notes-view.py --session /tmp/rake-dev-EXAMPLE` against a
visible development app with a selected note to check the hierarchy and private
editor focus targets.

See [test coverage](internal/test-coverage.md) for what's automated, and the [real-browser procedure](internal/notes-browser-testing.md) for Chrome input, long-document scrolling, background autosave, stale tabs, malformed recovery and a renderer crash.
