# Agent automation

Implementation plan (one commit per step):

1. Native CDP mouse, wheel, keyboard and text input, with explicit window selection.
2. Native `Page.captureScreenshot`, with bounded output and capture metadata.
3. Compact semantic snapshots and queries over retained inspection identities.
4. Bounded condition waits, distinguishing accepted actions from rendered results.
5. Capability discovery, a small agent client, and a complete native workflow test.

Reuse the existing development-only capture/control boundary. Actions never retry
uncertain mutations. Missing engine data and unsupported platforms remain explicit.
Standard CDP supports Chrome; Rake extensions supply native window identities,
semantic queries and render-aware waits without a JavaScript interpreter.

## Native input

On macOS, an opted-in inspection host exposes `Rake.getWindows` and
`Rake.selectWindow { windowId }`. Exactly one available window can be selected
automatically; multiple windows require explicit selection. Selection is local
to the connection and resets on runtime replacement.

`Input.dispatchMouseEvent` supports press/release/move (including dragging with
`buttons`) and pixel wheel deltas. `Input.dispatchKeyEvent` supports keyDown,
rawKeyDown, keyUp and char; `Input.insertText` sends literal Unicode to the focused
GPUI input handler. These are application events, not temporary inspection edits.
The adapter never moves the desktop cursor or sends keys to another application.

Coordinates use GPUI content logical points, rounded to whole points by the native transport. Text is bounded to 16 KiB.

- **Supported:** left, right and middle buttons, click counts up to three, and Alt, Control, Meta and Shift.
- **Unsupported:** multiple simultaneous mouse buttons, modified wheel gestures and IME composition.

Window identity and viewport geometry are validated again at runtime admission, and an uncertain action is never retried automatically.

## Screenshots

Start `rake cdp --session SESSION --capture --no-open --json` to prepare capture
once for the server. This opt-in keeps ordinary inspector startup free of Swift
compilation and screen-recording setup. It requires macOS 14+, Xcode command-line
tools and Screen Recording permission. `Rake.getCaptureStatus` reports helper
preparation failures; permission is checked when capture runs.

`Page.captureScreenshot { format: "png" | "jpeg", quality?: 0..100 }` returns base64 `data` plus `rake` metadata: native window and content rectangles, pixel dimensions, scale, timestamp and the separately sampled inspection generation.

It captures the selected whole native window, including chrome, on the active desktop, and doesn't claim exact GPU-frame or tree correlation.

- Viewport clipping and capture beyond the window are rejected.
- Resizing or replacement during capture invalidates the result.
- Images fit 1920×1440 and a 750 KiB encoded budget. Oversized PNGs report an error suggesting JPEG or a smaller window.

The server owns the temporary helper; capture/compiler subprocesses have finite
deadlines, bounded output and process-group cancellation. No image is captured
and no helper is compiled in production builds or ordinary CDP startup.

## Semantic snapshots

`Rake.getSnapshot` and `Rake.findElements` return compact captured nodes for the
selected window. `query` accepts exact `id`, `stableId`, `nodeId`, `name`,
case-insensitive `role`, literal `textContains`, `enabled` and `visible` filters.
Unknown filters fail explicitly. Defaults return at most 100 nodes; `limit` is
bounded to 256. Continue with `offset` and the returned revision as
`expectedRevision`; changed/missing revisions reject continuation.

Results include runtime and window identity, draw sequence, capture and metadata revision, node, parent and stable IDs, type, role, name, text, sampled bounds and clipping, enabled and focus state, and workspace-confined Rust source URLs.

Text previews stop at 512 UTF-8 bytes without splitting characters, and `textTruncated`, omission counts and engine coverage stay explicit. Detailed text and styles use the existing DOM and CSS methods. Visibility describes captured layout, clipping and accessibility, so final GPU occlusion and opaque descendants are outside coverage.

The runtime pins all pages to one inspection draw for five seconds, with at most four retained page leases shared with ordinary tree paging. Unchanged semantic indexes reuse the shared immutable cache, and a separate draw sequence advances without invalidating stable IDs or unchanged-tree caching. Switching windows retires old CDP node handles, and runtime replacement clears selection.

## Condition waits

`Rake.waitFor { query, state?: "present" | "absent", timeoutMs?: 1..30000,
afterFrame?: "decimal-sequence" }` creates a connection-owned wait and performs
one observation. The default timeout is five seconds. Poll
`Rake.getWait { waitId }`; `Rake.cancelWait { waitId }` cancels observation without
undoing or cancelling the original application action.

Queries also support exact `text` and `bounds` predicates. Bounds can specify `x`, `y`, `width`, `height` and a nonnegative `tolerance` (default 0.5 logical points, maximum 100). Combine them with role, name, ID, enabled and visible filters.

`afterFrame` requires a later inspection draw even if the tree identity is unchanged. It's an inspection-paint marker, not a GPU presentation fence or proof that async app work has finished.

Statuses are `pending`, `satisfied`, `timed_out`, `cancelled` and `invalidated`. Responses keep the last bounded observation, its revision and draw sequence, and any observation error.

- There are at most 32 handles. Completed or expired ones can be evicted for new waits, but pending ones never are.
- Selection and runtime changes invalidate pending waits.
- Absence refers to the captured tree, and can't succeed while its omission count is nonzero.

Each poll uses the normal finite request deadline. No long wait holds the worker,
and no fixed sleep substitutes for a readiness condition.

## Agent client and capabilities

`Rake.getCapabilities` reports the live runtime identity, available agent methods, source-read transport, numeric budgets and coverage and acknowledgement semantics. It's an agent operation catalog, not a percentage of upstream CDP.

Screenshots need `--capture`, a prepared helper and an available native window, and Screen Recording permission is checked at capture time. Errors stay explicit through `Rake.getCaptureStatus` and `Page.captureScreenshot`.

All methods accept an optional `rakeRuntimeId` precondition. Input additionally
accepts `rakeWindowId` and `rakeCapture`; a failed owner/capture precondition returns
`-32010`. Capture-guarded pointer input checks the exact generation again on the
native UI thread. A stale-reference rejection admits no input; transport failure
or a lost acknowledgement has an uncertain outcome and must not be replayed.

The dependency-free Node 22+ client is `crates/rake-cli/scripts/rake-agent.mjs`, with TypeScript exports in `crates/rake-cli/scripts/rake-agent.d.mts`.

- It discovers exactly one runtime (or requires `targetId`) and selects exactly one window (or requires `windowId`).
- It rejects ambiguous queries, and resolves references by stable identity with no name-based fallback.
- Input stays inside the native app, so the desktop cursor is unaffected.
- Only numeric loopback servers and same-origin, redirect-free source reads are accepted.
- Requests, source responses, pending work and timeouts have byte and count budgets.
- Disconnects and timeouts retire the client and never replay mutations.

```js
import {RakeAgent} from './crates/rake-cli/scripts/rake-agent.mjs';
const app = await RakeAgent.connect('http://127.0.0.1:PORT/TOKEN');
try {
  const button = await app.find({role: 'button', name: 'Increment counter'});
  const action = await app.click(button);
  await app.waitFor({text: 'Count: 1'}, {afterFrame: action.beforeFrame});
  const {bytes, metadata} = await app.screenshot({format: 'png'});
  const source = await app.source(button);
} finally {
  app.close();
}
```

Rendered text can be a separate Text boundary below its paragraph or button. Text predicates match captured text boundaries, so use `parentId` to relate them to enclosing controls.

Editable control values aren't harvested, though an app can deliberately render an echo or status value, as the agent fixture does. Snapshot previews are bounded, and the DOM text methods give detail.

The bundled CLI accepts one JSON action; arguments are passed separately from
its fixed JavaScript program. JSON goes to stdout, failures to stderr:

```sh
rake agent --url http://127.0.0.1:PORT/TOKEN '{"action":"find","query":{"id":"agent-increment"}}'
rake agent --url http://127.0.0.1:PORT/TOKEN '{"action":"click","query":{"role":"button","name":"Increment counter"}}'
rake agent --url http://127.0.0.1:PORT/TOKEN '{"action":"waitFor","query":{"text":"Count: 1"}}'
rake agent --url http://127.0.0.1:PORT/TOKEN '{"action":"screenshot","path":"/tmp/native.png"}'
```

Other actions: `capabilities`, `windows`, `snapshot`, `hover`, `scroll`, `drag`, `key`, `typeText` and `source`. `--window ID` and `--target ID` resolve ambiguity.

Use a persistent client connection when a workflow needs retained references, wait cancellation or a selected window. Run high-level actions sequentially, because a failed multi-event gesture can have partially run. Condition waits observe captured state, not app-wide quiescence, GPU presentation or network idle.

## Verification

```sh
cargo build -p rake-app-cli -p rake-agent-example --features rake-agent-example/devtools
node --test scripts/test-agent-client.mjs
node scripts/test-agent-cdp.mjs
```

The native suite needs an unlocked macOS 14+ desktop, Screen Recording permission, Node 22+ and Xcode command-line tools. It launches its own fixture and local adapter, closes only its own processes, and leaves logs and the image in the printed private temporary directory.

It doesn't claim arbitrary browser or Puppeteer API compatibility, or GPU-engine coverage beyond the reported inspection boundaries.

When macOS occludes the window, the suite supplies harmless pointer moves to request native draws, and makes no occluded-window frame-rate claim. The fixture uses F3 to resize, F4 for 24 timer-driven updates, and `RAKE_AGENT_ROUTER=1` for router mode. See [test coverage](internal/test-coverage.md) for what's exercised.

## Evidence bundles

`Rake.getEvidence { query?, limit?: 1..64, screenshot?: true, diagnosticsAfter?: 0 }` collects one bounded bundle. The SDK exposes `app.evidence(options)`, and the CLI action is `evidence`.

A query selects exactly one element. Without one, Chrome's current inspected node is used. The tree and selection share a pinned inspection draw, while Rust source (seven lines around the construction site), retained logs and network errors, and a JPEG are sampled separately and labelled so. The request uses the normal two-second total deadline, and never waits for app idle or retries a mutation.

Sections report `ok`, `partial` (data present with explicit loss counts), `error`, `truncated` (byte budget exceeded) or `omitted` (screenshot disabled). `complete` means requested sections were available without reported loss, not complete GPU coverage. An unavailable screenshot or missing selection leaves other sections usable.

- The whole response stays below 900 KiB through per-section budgets.
- Diagnostics keep at most 16 logs and 16 HTTP error records, expose loss and sequence, omit HTTP bodies, and leave Console and Network cursors untouched.
- Source reads keep the existing workspace and Rust-file restrictions.

The native suite issues an actual request to its own local HTTP 503 fixture and
verifies that the bundle contains the resulting diagnostic, failure metadata,
Rust source, tree/selection revision and a native image. Missing selections and
truncated tree prefixes remain explicit partial evidence.

## Actionability diagnostics

`Rake.getActionability { query }` resolves exactly one captured element and reports `canDispatch`, a whole-point dispatch location, `blockers`, `notes`, runtime and window identity, and the checked revision and draw. The SDK exposes `app.actionability(reference)` and runs it before pointer actions.

Known blockers raise `ActionabilityError` before mouse dispatch: hidden or disabled elements or ancestors, missing or zero geometry, full clipping, or a visible area with no valid integer input point. Partial clipping uses the visible intersection and stays a note. Missing clipping or enabled metadata stays explicit uncertainty.

The report includes the runtime picker's captured hit and its relationship to the target. Another painted boundary produces `possibly_covered`, and occlusion stays `unknown`, because opaque native hitboxes, pointer pass-through and final GPU occlusion aren't exposed. That hint doesn't block input.

WindowServer's whole-window occlusion is informational, because synthetic input can still drive an occluded app. A check is a sampled preflight, so guarded admission and a condition wait are still needed to observe the real result.

The native suite verifies disabled-target rejection, clipped rows, and a captured
overlay whose pointer interception remains unknown. CPU/client tests cover hidden
states, fractional edge coordinates and the absence of native mouse dispatch when
a known blocker is present.

## Subtrees, enclosing text and structured failures

`Rake.getSnapshot` and `Rake.findElements` accept `rootNodeId` to include one retained node and its descendants before applying the query. The root must belong to the selected window. Pagination still needs `expectedRevision`, and a changed revision restarts the read.

The SDK resolves stable references with `app.subtree(reference, query?, options?)`. CLI `subtree` uses `query` to select the root and an optional `filter` for descendants. Scoped waits and evidence accept the same `rootNodeId`, and removing a wait's root invalidates it.

Every projected element includes `textContent`, `textContentTruncated` and `textContentComplete`. `textContent` aggregates captured text children in tree order, so a paragraph can expose its text even when its direct `text` is null. A boundary's own rendered text takes precedence, to avoid counting text twice.

The preview stops at 512 UTF-8 bytes. Query filters `textContent` and `textContentContains` inspect the retained captured text rather than that preview, and exact matching needs complete captured metadata. `text` and `textContains` keep their direct-boundary meaning.

Aggregation uses one bounded shared text buffer and subtree ranges, with iterative traversal. Captured text has a 16 MiB total bound.

Omitted or truncated metadata keeps completeness false, and a `queryComplete: false` result can't satisfy an absence wait. These flags refer to captured boundaries, so opaque engine descendants and private editable values stay outside coverage.

```js
const count = await app.find({id: 'agent-count', textContent: 'Count: 1'});
const branch = await app.subtree(count);
await app.waitFor({textContent: 'Count: 1'}, {rootNodeId: count.nodeId});
```

CDP errors add `data.category`, `recovery`, `outcome` and `automaticReplay: false`. The client exposes them on `AgentError` and `ProtocolError`, and CLI failures emit one JSON error on stderr with a nonzero exit status.

- **Categories:** `element_not_found`, `ambiguous_element`, `node_stale`, `capture_changed`, `runtime_changed`, `window_changed`, `snapshot_changed`, `not_actionable`, `unsupported_operation`, `request_timed_out` and `disconnected`.
- **Recoveries:** `narrow_query`, `resolve_again`, `restart_pagination`, `select_window` and `reconnect`. These hint at an explicit next step, and never trigger automatic mutation retries.

| `outcome` | Meaning |
| --- | --- |
| `rejected` | A known failed admission or preflight |
| `read_failed` | An observation failure |
| `unknown` | Uncertainty after a transport failure, or an error with no authoritative admission result |
| `observation_only` | A wait timeout, which says nothing about whether the earlier action happened |

A multi-event gesture can partially run before a later event fails. The capability catalog reports the supported query fields and error contract version.
