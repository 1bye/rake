# Alpha 1 browser and release evidence

Records for `v0.1.0-alpha.1`. They do not qualify later revisions.

## Alpha verification evidence

The launch scope is macOS and browser source applications. Release evidence must
identify the exact commit and distinguish compilation, automated host tests and
real GUI observations. See the release's accompanying evidence for final-commit
readiness results; historical passes below are not final-commit verification.

The Fieldnotes module-loading fix was verified at
`af0cd47b55a4a8b2952dc7d5ae712bc34e130cc0`. All five local readiness groups,
the temporary CLI install and Web development/control flow passed at that revision.
The [complete browser checklist](#alpha-preparation-rerun-2026-11-03)
records actual Chrome 156.0.8078.80 Guest, Safari 26.6.2 and Firefox 159.0 runs on
macOS 26.6.2, including exact stored text, hidden autosave, stale writers,
interrupted saves, write failure/retry and malformed-checkpoint recovery.
Safari's initial hidden-writer discovery failure and successful retry are recorded.

Real Windows and Linux GUI runs, signing, notarization and installers were not
verified for this source alpha. Browser automation does not establish every IME,
accessibility, mobile input or execution during browser suspension. Hosted
compilation/boundary results establish only the checks actually run. Each release
must retain its own final-commit evidence and the limits of any reused manual run.

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
during this rerun. The table above records the historical observations and tested build identity;
it does not claim a rerun on later release commits. All test servers and
owned browser processes were stopped. The limitations listed above still apply.
