# Fieldnotes real-browser checks

Run these in a dedicated localhost origin with disposable test notes. Chrome
Guest mode isolates the test from personal profiles; its storage survives tab
reloads and renderer crashes, but closing all Guest windows intentionally deletes
that guest session's data. Its crash procedure below concerns a renderer crash;
the Safari and Firefox procedures interrupt the whole test browser.

```sh
cargo run -p rake-cli -- build --target web --package rake-notes --output /tmp/fieldnotes-web-check
python3 scripts/serve-web.py --example notes --directory /tmp/fieldnotes-web-check --control --port 0
```

Open the server's private launch URL in each browser under test. The app must
render its WebGPU canvas and show a saved library without experimental browser
flags. Safari and Firefox can use a new, dedicated localhost origin; use only
test windows if the procedure will force quit the whole browser. Use
`rake instances --json` to obtain each Web runtime ID; replace `FIRST` and
`SECOND` below with those IDs. A full reload or
renderer crash creates a new runtime ID. Use the matching installed Wasm bindings
tool described in [Fieldnotes](notes.md).

## Editing and autosave

1. Use New note, type a title, press Tab, and type body text with Enter between
   paragraphs. Title and body must remain independent fields.
2. Select the body and paste multiline text containing `café`, `日本語`, `😀` and
   `👩🏽‍💻`. Text and line breaks must appear exactly once; the CJK and emoji glyphs
   must render. Observe the completed paste before sending Undo, and observe
   Undo before sending Redo. Clipboard delivery is asynchronous; sending all
   three together can undo an earlier edit before the paste arrives.
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
For a deterministic stale-writer test, cancel only `SECOND`'s
`fieldnotes.autosave` task, then create its disposable local note. This prevents
its own checkpoint from winning the race before the first tab's write.
Keep the second tab selected while checking background work, so `FIRST` reports
hidden host visibility.

```sh
rake ctl --instance FIRST commands run notes.create --args '{"title":"Background test","body":"Must survive a stale writer."}'
rake ctl --instance FIRST --json snapshot
```

The first tab should become clean through autosave,
without Save now, while the browser permits it to execute. Then run `notes.save`
in `SECOND`. Its outdated library revision must produce a conflict, retain its
local dirty text, and preserve the first tab's committed library. Reload the
stale test tab, accepting any warning for its disposable unsaved note, and verify
that the first tab's content survived. Browser commands can report a failed
operation even when the CLI exits successfully; inspect the returned state.

The cancellable Web runtime driver uses the browser's microtask executor. GPUI's
chained-timer dispatcher can be throttled heavily in hidden tabs. CLI identity
probes honor `--timeout`; a hidden runtime must not be rejected by an unrelated
fixed two-second probe. Browser process suspension can still stop execution.
In the Safari run below, hidden-tab control requests timed out and selecting
the tab restored responsiveness. Perform the stale-writer test with each writer
selected when necessary. Host visibility does not guarantee background execution;
the cause of these Safari control timeouts was not isolated in this run.

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

For Safari and Firefox, force quit the test browser after acknowledging and
inspecting the checkpoint. On macOS, raise that browser's test window and hold
Shift-Option-Command-Escape, then relaunch it and restore the same origin. This
interrupts the whole browser without a normal application save; it is distinct
from Chrome's renderer-only crash test. Do not use an ordinary graceful quit as
evidence of crash recovery.

To exercise a failed main write without exhausting the machine's storage, run
this in the dedicated origin's Console:

```js
window.fieldnotesOriginalSetItem = Storage.prototype.setItem;
Storage.prototype.setItem = function (key, value) {
  if (key === 'dev.rake.fieldnotes.library') {
    throw new DOMException('Injected test quota failure', 'QuotaExceededError');
  }
  return window.fieldnotesOriginalSetItem.call(this, key, value);
};
```

Create a test note and attempt `notes.save`. The operation must report the
failure, the entry must remain dirty, main must retain the old snapshot, and
recovery must contain the new note. Restore
`Storage.prototype.setItem = window.fieldnotesOriginalSetItem` in the same
Console, retry Save, and verify that the entry becomes clean and both snapshots
contain the note. This injects a storage error; it does not measure the browser's
actual quota or private-browsing retention policy.

For the malformed-checkpoint regression, keep a valid saved main snapshot and
replace only the recovery value in this test origin with a JSON copy whose
revision is larger but whose first note ID is zero. Reload. The valid main data
must remain available, the damaged-save notice must appear, and a subsequent
save must repair recovery. Invalid document data must not be mistaken for a
concurrent valid writer merely because its revision is larger.

## Historical verification: 2026-10-24

On 2026-10-24, these cases were exercised through Chrome's actual macOS guest
window, WebGPU canvas, DOM input and `localStorage`/Web Locks, using the shared
runtime control API for task cancellation and command assertions. The crash
fixture existed only in recovery at revision 45 while main remained revision 44;
Chrome's renderer was crashed and the fixture recovered after Reload. Background
autosave completed in 2.76 seconds; a stale second tab was rejected and its reload
retained the accepted first-tab content.

The same built application was then exercised on macOS 26.6.2 through Safari
26.6.2 and Mozilla's signed Firefox 158.0.1 release. Actual browser windows,
WebGPU rendering, DOM input, Web Locks and `localStorage` were used; shared
runtime commands staged the deterministic persistence fixtures.

| Case | Safari 26.6.2 | Firefox 158.0.1 |
| --- | --- | --- |
| Independent title/body, Enter, Tab, multiline Unicode paste, undo/redo | Passed | Passed |
| 80-line document, selection collapse, prefix and last-line edits | Passed | Passed |
| Autosave and exact stored text after reload | Passed | Passed |
| Stale writer rejected; local dirty text retained; accepted text survives reload | Passed | Passed |
| Recovery-only note survives forced browser quit, including selected document and Unicode body | Main 96 / recovery 97; restored to both at 98 | Main 93 / recovery 94; restored to both at 95 |
| Injected main-write failure preserves old main/new recovery; explicit retry succeeds | Main 98 / recovery 99; retry committed 99 | Main 95 / recovery 96; retry committed 96 |
| Invalid document ID in a checkpoint with revision increased by 1000 | Repaired both snapshots to 100 | Repaired both snapshots to 97 |

Firefox background autosave completed in 1.64 seconds with hidden host visibility.
Safari's hidden-tab control requests timed out, then resumed when selected;
foreground autosave and subsequent recovery passed. No additional editing or
persistence defect was reproduced in these runs after the Chrome fixes.

This complements the Rust/native persistence suite and the Node adapter tests.
It does not establish mobile input, every operating-system IME, power-loss
durability of browser storage, actual quota exhaustion, or execution while the
browser suspends a page.

## Alpha preparation rerun: 2026-11-03

The complete checklist above was rerun on clean
`af0cd47b55a4a8b2952dc7d5ae712bc34e130cc0` (`codex/public-alpha-prep`), after
the Fieldnotes module-loading fix. The same built application ran on macOS
26.6.2 (25G83), Apple Silicon, in Chrome 156.0.8078.80 Guest, Safari 26.6.2,
and Mozilla's signed Firefox 159.0. No experimental browser flags were enabled.
Each browser used a separate disposable localhost origin. Actual browser input,
canvas rendering, Console storage assertions, Web Locks and shared runtime
commands were exercised; compilation alone was not counted as a browser pass.

| Case | Chrome 156 | Safari 26.6.2 | Firefox 159 |
| --- | --- | --- | --- |
| Independent title/body, Tab and Enter paragraphs | Passed | Passed | Passed |
| DOM multiline Unicode paste, separately observed undo and redo | Passed | Passed | Passed |
| Selection collapse, prefix, 80 numbered lines, Home/End and caret scrolling | Passed | Passed | Passed |
| Exact serialized text and selected document after autosave/reload | Main/recovery 78 | Main/recovery 140 | Main/recovery 80 |
| Autosave completes with hidden host visibility | Passed | Passed on retry; see below | Passed |
| Stale save rejected, exact local dirty text retained, accepted text survives reload | Passed | Passed | Passed |
| Recovery-only Unicode note and selection survive interruption | Renderer crash: 81/82 → 83/83 | Forced quit: 148/149 → 150/150 | Forced quit: 81/82 → 83/83 |
| Injected main-write failure retains old main/new recovery; explicit retry becomes clean | 83/84; retry 84 | 150/151; retry 151 | 83/84; retry 84 |
| Higher-revision recovery with note ID zero preserves the exact valid library and is repaired | 471/1471 → 472/472 | 151/1151 → 152/152 | 84/1084 → 85/85 |

Revision pairs in the last three rows are main/recovery. The recovery and damaged-save
notices were observed. Failure injection was removed before retry. Safari's process
absence was also checked between forced quit and relaunch.

Safari initially dropped the hidden writer from development discovery; its command
reported `no matching development instance; use rake instances --json`. Selecting
the writer restored control. A fresh synchronized-tab retry then completed autosave
while reporting `Hidden`, rejected the stale writer, and preserved the accepted
content after reload. This demonstrates execution when Safari permits it, without
promising execution during browser suspension.

Intermittent desktop capture/focus failures interrupted the harness. Affected
disposable fixtures were recreated and the cases repeated before counting them.
No additional application defect was reproduced and no application source changed
during this rerun. Build identity/checksums, command results, per-browser observations,
interruption process checks and cleanup evidence are retained outside the repository
in the `rake-fieldnotes-browser-checklist` evidence bundle. All test servers and
owned browser processes were stopped. The limitations listed above still apply.
