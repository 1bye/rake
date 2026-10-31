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

Coordinates use GPUI content logical points, rounded to whole points by the
existing native transport. Left/right/middle buttons, click counts up to three
and Alt/Control/Meta/Shift are supported. Multiple simultaneous mouse buttons,
modified wheel gestures and IME composition remain explicitly unsupported.
Text is bounded to 16 KiB. Window identity and viewport geometry are validated
again at runtime admission; an uncertain action is never automatically retried.

## Screenshots

Start `rake cdp --session SESSION --capture --no-open --json` to prepare capture
once for the server. This opt-in keeps ordinary inspector startup free of Swift
compilation and screen-recording setup. It requires macOS 14+, Xcode command-line
tools and Screen Recording permission. `Rake.getCaptureStatus` reports helper
preparation failures; permission is checked when capture runs.

`Page.captureScreenshot { format: "png" | "jpeg", quality?: 0..100 }` returns
base64 `data` and additional `rake` metadata: native window/content rectangles,
pixel dimensions, scale, timestamp and the independently sampled inspection
generation. It captures the selected whole native window, including window
chrome, on the active desktop. It does not claim exact GPU-frame/tree correlation.
Viewport clipping and capture beyond the window are rejected. Resize/replacement
during capture invalidates the result. Images fit 1920×1440 and a 750 KiB encoded
image budget; oversized PNGs report an error suggesting JPEG or a smaller window.

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

Results include runtime/window identity, draw sequence, capture/metadata revision,
node/parent/stable IDs, type, role/name/text, sampled bounds/clipping, enabled/focus
state and workspace-confined Rust source URLs. Text previews stop at 512 UTF-8
bytes without splitting characters; `textTruncated`, omission counts and engine
coverage remain explicit. Detailed text/styles use the existing DOM/CSS methods.
Visibility describes captured layout, clipping and accessibility; final GPU
occlusion and opaque descendants remain outside coverage.

The runtime pins all pages to one inspection draw for five seconds, with at most
four retained page leases shared with ordinary tree paging. Unchanged semantic
indexes reuse the existing shared immutable cache; a separate draw sequence
advances without invalidating stable control IDs or unchanged-tree caching.
Switching windows retires old CDP node handles; runtime replacement clears selection.

## Condition waits

`Rake.waitFor { query, state?: "present" | "absent", timeoutMs?: 1..30000,
afterFrame?: "decimal-sequence" }` creates a connection-owned wait and performs
one observation. The default timeout is five seconds. Poll
`Rake.getWait { waitId }`; `Rake.cancelWait { waitId }` cancels observation without
undoing or cancelling the original application action.

Queries also support exact `text` and `bounds` predicates. Bounds can specify
`x`, `y`, `width`, `height` and a nonnegative `tolerance` (default 0.5 logical
points, maximum 100). Combine them with role/name/ID, enabled and visible filters.
`afterFrame` requires a later inspection draw even if the tree identity stays
unchanged. It is an inspection-paint marker, not a GPU presentation fence or a
claim that every asynchronous application task has completed.

Statuses are `pending`, `satisfied`, `timed_out`, `cancelled` and `invalidated`.
Responses retain the last bounded observation, its revision/draw sequence, and
any observation error. There are at most 32 handles; completed/expired handles
can be evicted when new waits are created. Pending handles are never silently
evicted. Selection/runtime changes invalidate pending waits. Absence refers to
the captured tree and cannot succeed while its omission count is nonzero.

Each poll uses the normal finite request deadline. No long wait holds the worker,
and no fixed sleep substitutes for a readiness condition.

## Agent client and capabilities

`Rake.getCapabilities` reports the live runtime identity, available agent methods,
source-read transport, numeric budgets and coverage/acknowledgement semantics.
This is an agent operation catalog, not a percentage of upstream CDP. Screenshot
support requires `--capture`, a prepared helper and an available native window;
Screen Recording permission is checked at capture time. Capture errors remain
explicit through `Rake.getCaptureStatus` and `Page.captureScreenshot`.

All methods accept an optional `rakeRuntimeId` precondition. Input additionally
accepts `rakeWindowId` and `rakeCapture`; a failed owner/capture precondition returns
`-32010`. Capture-guarded pointer input checks the exact generation again on the
native UI thread. A stale-reference rejection admits no input; transport failure
or a lost acknowledgement has an uncertain outcome and must not be replayed.

The dependency-free Node 22+ client is `scripts/rake-agent.mjs`, with TypeScript
exports in `scripts/rake-agent.d.mts`. It discovers exactly one runtime (or requires
`targetId`), selects exactly one window (or requires `windowId`), rejects ambiguous
queries, and resolves references by stable identity without a name-based fallback.
Input stays inside the native application; the desktop cursor is unaffected.
Only numeric loopback servers and same-origin, redirect-free source reads are
accepted. Requests, source responses, pending work and timeouts have byte/count
budgets. Disconnects/timeouts retire the client and never replay mutations.

```js
import {RakeAgent} from './scripts/rake-agent.mjs';
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

Rendered text can be a separate Text boundary below its paragraph/button. Text
predicates match captured text boundaries; use `parentId` to relate those nodes
to enclosing controls. Editable control values are not harvested. An application
can deliberately render a separate echo/status value, as the agent fixture does.
Snapshot text previews are bounded; the existing DOM text methods provide detail.

The bundled CLI accepts one JSON action; arguments are passed separately from
its fixed JavaScript program. JSON goes to stdout, failures to stderr:

```sh
rake agent --url http://127.0.0.1:PORT/TOKEN '{"action":"find","query":{"id":"agent-increment"}}'
rake agent --url http://127.0.0.1:PORT/TOKEN '{"action":"click","query":{"role":"button","name":"Increment counter"}}'
rake agent --url http://127.0.0.1:PORT/TOKEN '{"action":"waitFor","query":{"text":"Count: 1"}}'
rake agent --url http://127.0.0.1:PORT/TOKEN '{"action":"screenshot","path":"/tmp/native.png"}'
```

Other actions: `capabilities`, `windows`, `snapshot`, `hover`, `scroll`, `drag`,
`key`, `typeText` and `source`; `--window ID` and `--target ID` resolve ambiguity.
Use a persistent client connection when a workflow needs retained references,
wait cancellation or a selected window. Execute high-level actions sequentially;
a failed multi-event gesture can have partially run. Condition waits observe
captured state, not application-wide quiescence, GPU presentation or network idle.

## Verification

```sh
cargo build -p rake-cli -p rake-agent-example --features rake-agent-example/devtools
node --test scripts/test-agent-client.mjs
node scripts/test-agent-cdp.mjs
```

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
