# Test coverage

What each suite exercises, and what it does not establish. Moved here from the user guides.

## Routing 

Routing tests use real destination codecs, local futures, operation scheduling,
guards and a recorded history sink. A separate resource test uses local stand-ins
to prove retention and mount-failure rollback. GPUI mounting and the actual History
API were additionally exercised in the native and browser examples. The portable
`windows` suite uses a physical-host double for scheduling, close guards, cancellation,
visibility and restoration. `python3 scripts/test-windows.py` separately exercises
real macOS GPUI windows, scoped CLI navigation, closing the original/last host,
reopening and checkpointed multi-window restart. Other native platforms remain
unverified.
 

## Guarded-Quit 

`cargo test -p rake --no-default-features --test guarded_quit` exercises public
portable behavior, including registered save commands, revisions, histories,
both job policies, cancellation, retries and reload exclusion. `cargo test -p
rake-cli --test runtime` includes real Unix-socket CLI decisions and quit status.
`node scripts/test-inspector-ui.mjs` checks actual panel code with DOM/transport
doubles. `python3 scripts/test-quit.py` is a separate macOS display test for real
native exit and pending HTTP-command coordination. Wasm compilation checks the
Web adapter; a real browser is required to verify its page lifecycle and warning.
 

## Storage 

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

## Notes 

Verification: portable persistence tests exercise actual files/leases/workers;
`scripts/test-notes.py` exercises a real macOS app and CLI through SIGKILL recovery
and graceful quit. `scripts/test-notes-storage.mjs` executes the actual browser
adapter functions against an in-memory storage/lock host, covering conflicts,
quota failures and recovery. It does not establish live browser/GPU behavior.
The separate [real-browser test procedure](notes-browser-testing.md) records
Chrome input, long-document scrolling, background autosave, stale tabs,
malformed recovery and an actual renderer crash.
 

## View syntax

Before heading composition, canonical events and the newer elements were added,
the example was smoke-tested on macOS: Enter and Command+I activation,
locked mouse/keyboard activation, Tab skipping disabled controls, native disabled
accessibility state, and row activation after a typed reorder. The component
follow-up also verified header/body/footer layout, dynamic Badge properties, and
retained row activation/focus after reordering inside Card. Those earlier macOS
checks also covered paragraph composition for nested bold/italic runs, dynamic
colored text, Unicode, single- and two-line ellipses, and complete untruncated
accessibility values.
The later events, tooltips, SVGs, semantic containers, rules, composed headings,
`match`, shorthand, defaults, unquoted text and markup-valued properties have compile, unit, UI and TestPlatform
frame coverage only, without native or browser interaction checks. Web compilation
is checked; browser input and screen-reader interaction have not been verified. 

Expansion snapshots preserve builder order and generated identities. Compiler UI
fixtures cover individual and combined errors, attribute recovery, legacy event
warnings and expression/listener type checking beside macro errors. Utility
coverage tests compare the documented list with the checked table and compile all
supported methods against GPUI. Runtime construction tests cover match arms,
booleans, shorthand, component defaults, semantic roles and SVG labels. A direct
rust-analyzer completion probe verified typed listener fields beside an invalid
utility; it did not modify the source files. 

The opt-in `view-test-support` feature enables GPUI's own deterministic TestPlatform.
Its frame tests exercise keyed element state, implicit button focus, retained
entity data/selection, explicit focus and scroll offsets across parent rerenders,
reordering and insertion. Component frame tests check forwarded root IDs, implicit
focus and keyed state across reorder/insertion, and retained child ownership.
Paragraph and heading frame tests cover shared inline flow, width changes, explicit
breaks, line limits, composed heading names and GPUI truncation. The TestPlatform
has no active accessibility adapter; heading node properties are inspected during
render alongside real layout measurements. Keyboard frame tests cover typed actions, Tab/Shift+Tab,
negative indices, consumed events, disabled mouse/keyboard activation, and
keyboard-visible styling. Pointer tests exercise right/middle buttons, hover and
wheel dispatch, including disabled suppression. The platform's window/text/GPU backend is a test double;
these checks do not establish native editor or browser input behavior. 

The example's opt-in `view-test-support` tests render the actual counter,
components, retained history, popover, document dialog and help view. Reviewed
text fixtures assert the exact distinct strings sent to GPUI text layout, with
no whitespace normalization in the assertions. Scenarios cover initial and
incremented counts, light/dark themes, locked/hidden history, open overlays and a selected virtual
document. The recorder keeps full composed text before truncation and omits
ellipsis measurement candidates; it does not assert pixel output, text order,
duplicate occurrences or native/browser rendering. CI runs these tests explicitly. 

## Runtime control

The native integration test uses real Unix sockets and real terminal child processes,
including disconnect/retry and private discovery. The broker checks use real
loopback HTTP and the compiled CLI, including token, Host/Origin validation,
redelivery, queue bounds and lease expiry. The Wasm browser smoke check opened a
Live tab from the CLI, retried without creating a duplicate, edited the GUI draft,
resolved Save through the CLI and observed the committed close and task cleanup.
 

## CDP agents

The native suite requires an unlocked macOS 14+ desktop, Screen Recording
permission, Node 22+ and Xcode command-line tools. It launches its own native
fixture and local adapter, then exercises the public SDK and bundled CLI:
capabilities → window selection → semantic lookup → guarded click → later-render
wait → Unicode typing/replacement → keyboard shortcut → drag → scroll/clipping
→ native PNG → Rust source snippet. It also checks ambiguity, missing nodes,
timeout/cancellation, stable identity, and stale runtime/window/capture rejection.
It closes only its own processes; logs and the native image stay in the printed
private temporary directory. This suite passed on a real macOS window, including
visual inspection of its PNG. It does not claim arbitrary browser/Puppeteer API
compatibility or GPU-engine coverage beyond the reported inspection boundaries. 

The native regression suite also checks resizing without changing stable IDs,
changing layouts across draws, reconnecting without replay, replacing the app
runtime, and two independently inspected router windows. Switching windows
invalidates pending waits and old element references cannot operate on the new
selection. Each window retains its own counter. The fixture uses F3 to resize,
F4 for 24 timer-driven updates, and `RAKE_AGENT_ROUTER=1` for router mode. When
macOS occludes the window, the suite supplies harmless pointer moves to request
native draws; it does not claim an occluded-window frame-rate measurement. 

## Chrome Console

- Reviewed commands entered through the Console prompt; helper namespace
  suggestions and expandable help/results appeared.
- Information, warning and error rows displayed their correct severity; Unicode
  command results and structured/causal fields were exposed for expansion.
- Clicking a log’s source link opened the real Rust file as read-only text.
- A failed result displayed `status: failed`; slow commands displayed
  `status: running` with a request key and observation token.
- Search found nine matches; Clear reported zero messages.
- Keep log retained previous failed/running results across a checkpointed restart
  and displayed the successor’s startup plus the restored-from notice.
- After clearing, the deliberate crash left the panic message, Rust backtrace,
  exit code 73 and disconnect notice visible in Chrome.

## Chrome Network

- Eight seeded request rows showed HTTP/transport statuses; Headers and Payload
  displayed repeated response headers, credential redaction, decoded queries and
  the Unicode JSON request body.
- Preview and Response displayed JSON; binary Response used the hex viewer and
  showed the exact `00 FF 80 2A` bytes.
- Initiator showed command/operation/span context. Clicking the source link opened
  the actual Rust file in Chrome. Timing showed the measured interval and the
  unsplit-phase limitation described above.
- `status-code:503` filtered eight requests down to one; response search found
  three matches in two requests. Copy as cURL and Copy response were pasted into
  an inert search field and checked, without executing the copied command.
- Clear removed all rows; only eight newly generated requests reappeared. Keep log
  retained those rows across checkpointed replacement, and an old response loaded
  afterward. New traffic increased the list to sixteen requests.
- Cancelled/timed-out responses displayed explicit incomplete-capture reasons.

The long-running Chrome pass also exposed repeated warnings for evictions of
already-consumed diagnostics. Development snapshots now distinguish history
eviction from unrecorded data; unread sequence gaps still warn. A regression test
was observed failing before the fix and passing afterward. A final Chrome pass
generated 104 requests and confirmed 1,188 history evictions while Console stayed
at eleven messages without the repeated loss warnings.

## Chrome Performance

- Record/Stop loaded a live native profile; the slow probe appeared at 26.04 ms.
- Search selected the callback and exposed stable identity, phase, inclusive
  measurement, construction location and an authenticated source link. Opening
  that link displayed the actual Rust fixture source.
- Imported JSON displayed draw, redraw, callback, command, task, span, HTTP and
  summary lanes. An imported 2.24-second profile showed about 345 ms of draw work,
  rather than counting async lifetimes in the main-thread total.
- A zero-draw profile loaded with a finite 0–37 ms range. After a live runtime
  replacement, Stop loaded the old runtime’s partial profile with a finite
  38.30-second range rather than waiting indefinitely.
- Save trace produced gzip JSON; Chrome loaded that export and its recent-session
  picker restored the earlier live recording. Clear returned to the live metrics page.

The focused CDP suite (73 tests), native GPUI recording/calibration test, diagnostic
and protocol tests, feature-enabled Clippy, production-disabled store tests,
rustdoc, formatting and the GPUI-enabled WASM compilation checks passed.
A separate no-default-features WASM check fails in the existing
`control/web.rs` references to GPUI-gated `reload::host`; those references predate
this work. This is not a claim of headless WASM support or a browser GPU runtime test.

## Chrome Memory

- Allocation sampling Start/Stop, call-tree search for `allocate_probe`, and the
  source link opening `memory_fixture.rs` at line 23, column 26 in Sources.
- A sampling profile exported from Chrome and parsed as a native allocation
  profile. Requested JavaScript heap usage remains explicitly zero.
- Baseline/retained/released heap snapshots. Comparison showed eight new samples
  and +8.4 MB, followed by eight deletions and −8.4 MB after release.
- Heap snapshot export and reopening the exported file in Chrome (2.9 MB of
  estimated remaining sampled allocations in that capture).
- Console summary/stop helpers, clearing profiles on stop, and checkpointed
  replacement clearing old profiles before a new capture without reconnecting.

All 85 CLI unit tests, allocator lifecycle/failure/retention/contention tests, the public native
allocator consumer, doctests, formatting and warning-free Clippy checks passed.
Disabled-feature native builds and the supported GPUI/WASM configuration compile;
WASM does not expose the native allocator. A separate no-GPUI WASM check hits the
existing `control/web.rs` references to the GPUI-gated reload host; this task does
not change that configuration. Native execution was verified on macOS, not Linux.

## Chrome Sources

Verified in stock Chrome 156.0.8078.74 on 2026-10-30: Sources navigator, Quick Open
with a line suffix, within-file search, global content search, read-only contents,
and Console/Network links to the corresponding Rust file and line. This includes
a Console file captured after DevTools connected, the Performance callback link
at `performance_fixture.rs:137:14`, and Quick Open after checkpointed replacement:
one current `lib.rs` entry, loading at line 597 without reconnecting. The final
native regression, all 81 CLI unit tests, formatting and warning-free CLI Clippy
checks passed. Rust resources are plain text;
Rust syntax highlighting, breakpoints, stepping, local variables and source edits
remain outside this adapter's coverage. The system editor association was reused,
without a new UI verification of external editor line navigation.

