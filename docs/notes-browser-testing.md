# Fieldnotes real-browser checks

Run these in a dedicated localhost origin with disposable test notes. Chrome
Guest mode isolates the test from personal profiles; its storage survives tab
reloads and renderer crashes, but closing all Guest windows intentionally deletes
that guest session's data. The crash test below concerns a renderer crash.

```sh
cargo run -p rake-cli -- build --target web --package rake-notes --output /tmp/fieldnotes-web-check
python3 scripts/serve-web.py --example notes --directory /tmp/fieldnotes-web-check --control --port 0
```

Open the server's private launch URL in Chrome. The app must render its WebGPU
canvas and show a saved library. Use `rake instances --json` to obtain each Web
runtime ID; replace `FIRST` and `SECOND` below with those IDs. A full reload or
renderer crash creates a new runtime ID. Use the matching installed Wasm bindings
tool described in [Fieldnotes](notes.md).

## Editing and autosave

1. Use New note, type a title, press Tab, and type body text with Enter between
   paragraphs. Title and body must remain independent fields.
2. Select the body and paste multiline text containing `café`, `日本語`, `😀` and
   `👩🏽‍💻`. Text and line breaks must appear exactly once; the CJK and emoji glyphs
   must render. Undo and redo must restore the whole paste.
3. Select all, press Left, and type a prefix. It must appear at the beginning.
4. Paste 80 numbered lines. The last line and caret must be reachable; Home/End,
   selection and scrolling must agree with the painted rows.
5. Let the status become Saved, reload, and verify the title, body and selected
   document. In DevTools Console, compare the actual serialized text and revision
   in `dev.rake.fieldnotes.library` and `dev.rake.fieldnotes.recovery`; a canvas
   screenshot alone does not prove what was stored.

Browser paste must reach GPUI through its DOM `paste` event. GPUI's synchronous
Web clipboard-read function returns no data; consuming the paste keydown prevents
that DOM event. Native paste continues to use the synchronous native clipboard.

## Background work and stale tabs

Open a second tab using the same private launch URL after the first has saved.
Keep the second tab selected, so `FIRST` reports hidden host visibility.

```sh
rake ctl --instance FIRST commands run notes.create --args '{"title":"Background test","body":"Must survive a stale writer."}'
rake ctl --instance FIRST --json snapshot
```

The first tab must become clean through autosave, without Save now. Then create
a different note in `SECOND` and run `notes.save`. Its outdated library revision
must produce a conflict, retain its local dirty text, and preserve the first
tab's committed library. Reload the stale test tab, accepting the warning for
its disposable unsaved note, and verify that the first tab's content survived.

The cancellable Web runtime driver uses the browser's microtask executor. GPUI's
chained-timer dispatcher can be throttled heavily in hidden tabs. CLI identity
probes honor `--timeout`; a hidden runtime must not be rejected by an unrelated
fixed two-second probe. Browser process suspension can still stop execution.

## Interrupted and damaged saves

For a deterministic interruption, list `FIRST`'s tasks and cancel only its
`fieldnotes.autosave` activity using the returned task ID. Create another test
note, then run:

```sh
rake ctl --instance FIRST commands run notes.save --args '{"checkpoint_only":true}'
```

In Chrome's Console, verify that the recovery revision is newer than the main
revision and that the new note exists only in recovery. Navigate that test tab
to `chrome://crash`, then use Chrome's Reload button. The app must recover the
checkpoint-only note, selected document and text, show a recovery notice, and
eventually complete a normal save. Confirm the new main snapshot in Console.

For the malformed-checkpoint regression, keep a valid saved main snapshot and
replace only the recovery value in this test origin with a JSON copy whose
revision is larger but whose first note ID is zero. Reload. The valid main data
must remain available, the damaged-save notice must appear, and a subsequent
save must repair recovery. Invalid document data must not be mistaken for a
concurrent valid writer merely because its revision is larger.

## Recorded verification

On 2026-10-24, these cases were exercised through Chrome's actual macOS guest
window, WebGPU canvas, DOM input and `localStorage`/Web Locks, using the shared
runtime control API for task cancellation and command assertions. The crash
fixture existed only in recovery at revision 45 while main remained revision 44;
Chrome's renderer was crashed and the fixture recovered after Reload. Background
autosave completed in 2.76 seconds; a stale second tab was rejected and its reload
retained the accepted first-tab content.

This complements the Rust/native persistence suite and the Node adapter tests.
It does not establish Firefox/Safari, mobile input, or every operating-system IME.
