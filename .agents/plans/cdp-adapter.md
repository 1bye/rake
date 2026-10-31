# Plan: Chrome DevTools Protocol (CDP) adapter

Handoff plan. The goal is to let Chrome's own DevTools frontend inspect a Rake
runtime. Requests, Console, Traces and later Elements go through a CDP server
that translates the existing protocol-1 runtime API. Rake-specific panels
(Workspace, Routes, Tasks, Operations, Decisions, Reload) stay in Rake's own
inspector. CDP has no equivalent for them.

Phase 0 is a **spike** with a hard gate. Later phases start only if it passes.

## Background (read once)

- Every runtime read goes through `Request`/`Reply` in
  `crates/rake/src/control/protocol.rs`. The adapter is one more client, like
  the CLI. It never mutates router state.
- `rake-cli` already reaches both transports. `exchange(instance, request, timeout)`
  in `crates/rake-cli/src/terminal.rs` (line ~376) handles `Endpoint::Local`
  (Unix socket) and `Endpoint::Web` (loopback broker, `/__rake/client`).
  Discovery is `rake::control::local::discover`.
- `Query::Diagnostics { after, kind }` returns `diagnostics::Snapshot`: a
  `Vec<Record>` plus the `dropped` and `output_dropped` counts. A `Record` has
  `sequence`, `elapsed_us` (monotonic, runtime-relative), `level`, `target`,
  `kind`, `span_id`, `parent_span_id`, `duration_us` and `fields`.
- Record kinds: `event` (tracing events), `span_open`, `span_close` (carries
  `duration_us`) and `request`.
- `request` records come from `Lifecycle::event` in `crates/rake/src/http.rs`
  (line ~590). Each has `request_id`, `phase`, `method`, a redacted `url`,
  `status`, `received_bytes` and `duration_us`. The phases are `started`,
  `headers_received`, `body_progress` (with `bytes`), `completed`, `failed`,
  `timed_out`, `cancelled` and `abandoned`.
- `Query::InspectTree {}` and `Query::InspectElement { id }` return the bounded
  element capture (`crates/rake/src/inspection.rs`). It holds at most 256 nodes.
  IDs are opaque and scoped to one capture. Each node carries `layout`, authored
  `styles` JSON and `source`.
- `RuntimeInfo` has no wall-clock start time. CDP's `wallTime` fields need one
  (see Phase 1).
- The current HTML inspector is served by `scripts/serve-web.py`, which
  `scripts/dev.py` launches. `rake-cli` embeds these scripts
  (`crates/rake-cli/src/project.rs`) and runs them with `python3`.
- Reference: the CDP spec at https://chromedevtools.github.io/devtools-protocol/
  (pin the "tot" revision you test against). React Native's Fusebox shows a
  non-browser runtime serving CDP to a stock frontend.

## Decisions (proposed; owner confirms before Phase 1)

- **Write the adapter in Rust, in `rake-cli`, as `rake cdp`.** It reuses the
  typed protocol and `exchange()` for both transports. It is unit-testable as
  pure translation and doesn't grow the Python host.
- **Use the stock frontend; no fork.** Target Chrome's bundled
  `devtools://devtools/bundled/devtools_app.html?ws=…`, plus `chrome://inspect`
  through `/json/list`. Forking `devtools-frontend` is out of scope.
- **Read-only first.** Phases 1–5 issue only queries. Phase 5b adds the only
  development-only commands (highlight and inspect mode), each with a lease. Any CDP method that would
  mutate (`Runtime.evaluate`, `CSS.setStyleTexts`, `DOM.setAttributeValue`)
  returns a CDP error until Phase 6 is explicitly approved.
- **Report gaps; don't invent data.** Headers and bodies aren't captured, so
  they stay empty or unavailable. Drops and gaps show up as Console warnings,
  matching the current inspector's honesty rules.
- **Dev only.** Bind to loopback only. The WebSocket path carries a random
  session token. Validate the `Host` header against the bound loopback
  `host:port`, which blocks DNS rebinding. Accept `Origin` only when it is
  absent or `devtools://devtools`. Production builds are untouched.
- **One CDP target per runtime instance.** When a runtime is replaced, the
  adapter ends the session cleanly in Phase 1 and follows `restored_from` in
  Phase 5.
- **New dependency.** Use `tungstenite` (sync, no async runtime) on a
  `std::net::TcpListener` with one thread per connection. Check its version,
  features and MSRV against `rust-toolchain.toml` before adding it.

## Before you start

1. Read `docs/devtools.md` (security and capture limits) and
   `docs/runtime-control.md`.
2. Build and run the routing example under `rake dev --target desktop`. Call
   `rake diagnostics` and `rake inspect tree` from the terminal so you know the
   real record shapes. `demo.fetch` produces request records and `demo.echo`
   produces logs.
3. Record the Chrome version you test with. All frontend-behaviour findings
   are tied to it.

## Phase 0: What does the stock frontend need? (spike, report only)

Throwaway code in a scratch binary or a `#[ignore]` test.

1. Serve `/json/version` and `/json/list`, and accept the WebSocket upgrade.
   Log every method the frontend sends on connect and reply `-32601` to all.
2. Try both target types in `/json/list`:
   - `type: "node"`, which opens `js_app.html`: Console, Sources and Memory only.
   - `type: "page"`, which opens `devtools_app.html`: Elements, Network and the
     other page panels.
3. Find the minimum replies the `page` frontend needs before Network and Console
   work. Expect at least `Page.getResourceTree` / `Page.getFrameTree` with one
   synthetic frame, `Runtime.runIfWaitingForDebugger`, and the `*.enable`
   acknowledgements.
4. Check how the frontend opens from the CLI. Does `open` (macOS) or
   `webbrowser.open` accept `devtools://` URLs, or must the user paste the URL
   or use `chrome://inspect`? Also try the hosted
   `chrome-devtools-frontend.appspot.com/serve_rev/@<rev>/devtools_app.html?ws=`.

**Gate:** with one fake `Network.requestWillBeSent` and `loadingFinished`, the
request appears in the Network panel and one `Log.entryAdded` appears in
Console. If the `page` frontend won't run without a real browser page, stop and
report. A `node` target with only Console is a much smaller win; the owner
decides whether to continue.

Write the findings (methods seen, minimum replies, how it opened, Chrome
version) to `.agents/reports/cdp-adapter-spike.md`.

## Phase 1: Server skeleton, discovery and lifecycle

Code: `crates/rake-cli/src/cdp/`

- `mod.rs` holds the `rake cdp` entry point. Flags: `--session`,
  `--instance UUID|APPLICATION_ID`, `--port` (default 9229 is not assumed
  free; report the bound port), `--no-open`, `--json`. Mirror `rake inspect`
  argument handling and `HELP` text.
- `server.rs` holds the listener, the Host/Origin/token checks, `/json/version`,
  `/json/list` (one entry per discovered instance) and the WebSocket upgrade.
  It runs one session thread per connection.
- `session.rs` tracks per-connection state: the selected instance, the
  diagnostics cursor, the enabled domains and a bounded outbound queue. Overflow
  drops events and counts them.
- `wire.rs` holds the owned CDP message types (`{id, method, params}`,
  `{id, result | error}`, `{method, params}`), with no stringly assembly at
  call sites.
- Poll loop: while at least one domain is enabled, call
  `Query::Diagnostics { after: cursor }` at a fixed interval (start at 250 ms;
  measure it) and translate. When the runtime is gone or the reply comes from a
  different `RuntimeId`, send `Inspector.detached { reason }` and close.
- Clock: add `started_at_unix_ms` to `RuntimeInfo` (this changes the protocol
  and needs a CLI parity test), or set `wallTime` from the adapter's own
  observation clock and document the skew. Pick one and record it in the
  report.

Tests: Host/Origin/token rejection, `/json/list` shape, unknown method → `-32601`,
and detach on a vanished runtime. Base them on the existing harness in
`crates/rake-cli/tests/runtime.rs` (real CLI child process, private socket,
headless router).

## Phase 2: Network domain

Code: `cdp/network.rs`. It is pure: `fn translate(&Record) -> Vec<Event>`.

| Rake `request` phase | CDP event | Notes |
| --- | --- | --- |
| `started` | `Network.requestWillBeSent` | `requestId = "<request_id>"`, `type: "Fetch"`, redacted `url`, `method`, `headers: {}`, `initiator.type: "other"` |
| `headers_received` | `Network.responseReceived` | `status`; empty `headers`; `mimeType: ""` |
| `body_progress` | `Network.dataReceived` | `dataLength = bytes` |
| `completed` | `Network.loadingFinished` | `encodedDataLength = received_bytes` |
| `failed` / `timed_out` | `Network.loadingFailed` | `errorText` = phase |
| `cancelled` / `abandoned` | `Network.loadingFailed` | `canceled: true` |

- `timestamp = elapsed_us / 1e6`, which is monotonic seconds as CDP expects.
- `Network.getResponseBody` and `getRequestPostData` return an error saying
  bodies are not captured.
- Span context (`span_id`) can go into `initiator` as a `stack`-less
  description. Don't fake a JS stack.

Tests: a table test from synthetic `Record`s to events for every phase,
including abandonment after `headers_received`. An integration test runs
`demo.fetch`-style traffic against a loopback HTTP server and asserts the event
sequence over a real WebSocket.

## Phase 3: Console (Log and Runtime domains)

Code: `cdp/console.rs`

- First, `kind == "event"` → `Log.entryAdded`. Level mapping: TRACE/DEBUG →
  `verbose`, INFO → `info`, WARN → `warning`, ERROR → `error`. `text` is the
  `message` field plus `key=value` for the other fields. `url` is the target.
- Then `Runtime.consoleAPICalled`, with fields as an object `RemoteObject` that
  has an inline `preview`. `Runtime.getProperties` serves expansions from a
  bounded per-session `objectId → Value` map; evicted IDs return an error.
- `Runtime.enable` sends one synthetic `executionContextCreated`
  (`name: "<application_id>"`).
- Loss: if `dropped` or `output_dropped` grows, or the cursor skips a sequence,
  emit a `warning` entry with the counts.
- Process stdout/stderr (the supervisor's `output.json` under `--session`)
  becomes `Log.entryAdded` with `source: "other"`, prefixed `[stdout]` or
  `[stderr]`. It has its own clock and claims no runtime ownership.

Tests: level mapping, object preview, `getProperties` after eviction, loss
warnings.

## Phase 4: Traces

Code: `cdp/trace.rs`

1. **File export first.** `rake cdp --export-trace FILE` (or
   `rake trace export`) writes Chrome trace-event JSON. Each span is a complete
   event (`ph: "X"`, `ts` from `span_open.elapsed_us`, `dur` from
   `span_close.duration_us`). Async-overlapping spans use `ph: "b"`/`"e"` with
   `id = span_id`. A span with a missing close is emitted as an instant event
   with an `args.incomplete` flag. This opens in Perfetto and in the
   Performance panel's "Load profile" with no CDP work at all.
2. **Live `Tracing.start`/`end` (stretch).** Buffer the span records between
   them, then send `Tracing.dataCollected` chunks (≤ 1 MiB) and
   `Tracing.tracingComplete`. Do this only if Phase 0 showed the Performance
   panel accepts a frame-less recording.

Tests: parent/child nesting, overlapping async spans, missing close, a
bounded-output check.

## Phase 5: Elements (DOM, CSS, Overlay) and runtime replacement

Code: `cdp/dom.rs`. Start only after Phases 1–3 are in use.

- `DOM.getDocument` synthesizes `#document` plus the tree from `parent` links.
  `nodeName` = `kind`. Attributes: `id` = `semantic_id`, and `data-source` =
  `file:line:col`. Map opaque capture IDs to integer `nodeId`s per capture.
  A new capture sends `DOM.documentUpdated`. Omitted nodes become one comment
  node, "N elements omitted".
- `DOM.getBoxModel` uses `layout` for every quad. GPUI doesn't report margin
  and padding separately, so say so and don't invent them.
- `CSS.getMatchedStylesForNode` turns the authored style JSON into an
  `inlineStyle` of CSS property names. Unmappable keys keep their GPUI name
  with a `-gpui-` prefix. `CSS.getComputedStyleForNode` returns only `width`
  and `height` from layout.
- `Overlay.highlightNode` and `setInspectMode` need new runtime support (see
  Phase 5b). Until 5b lands they return an error and the frontend's hover
  highlight does nothing.
- Runtime replacement: follow `restored_from` like the HTML inspector does.
  Send `Runtime.executionContextsCleared` and `DOM.documentUpdated` plus a Log
  entry ("restored from <old RuntimeId>"), and reset all cursors and ID maps.

**Gate:** Chrome's Elements panel shows the routing example's tree with styles
and box model. If its `page`-target assumptions make it unusable, keep Elements
in Rake's own inspector and record why.

## Phase 5b: Highlight and inspect-mode picking

Two directions, both on the existing `Overlay` domain. This phase changes the
protocol, so it touches `crates/rake` (runtime) and `rake-cli` (adapter), plus
CLI parity tests. These are development-only control commands, not
application commands. Neither one changes router state, and neither one gets
retained operation results.

### Runtime (`crates/rake`)

- **Highlight state.** It is per window and held by the inspection host. It
  stores an optional capture element ID, a capture generation and an expiry.
  Paint it at the window root, after the app, as a translucent fill plus a
  1 px border, with a label for `kind` / `semantic_id` and `width×height`. The
  window-root `OverlayHost` (`crates/rake/src/overlay/elements.rs`) is likely
  the right owner; confirm before choosing. Use the bounds already captured in
  prepaint (`crates/rake/src/inspection/view.rs`, around line 174). Don't
  re-measure.
- **New command** `Command::HighlightElement { id: Option<String>, generation }`.
  `None` clears the highlight. A stale generation is rejected with a typed
  error and never highlights the wrong node. A highlight expires after a short
  lease (start at 2 s) unless renewed, so a vanished client can't leave it on.
- **New command** `Command::InspectMode { enabled: bool, lease_ms }`. While it
  is enabled:
  - A window-level **capture-phase** mouse listener intercepts mouse moves and
    clicks, so the app receives none of them. Escape cancels the mode.
  - Hit test: the deepest boundary in the latest capture whose bounds contain
    the cursor, taking the last one in prepaint order. Intersect with the
    element's content mask (record it in prepaint next to the bounds) so that
    clipped or scrolled-away content can't be hit. Uninstrumented areas
    resolve to the nearest inspected ancestor.
  - Hovering updates the highlight. A click records the pick and leaves the
    mode.
  - The mode **ends by itself** when its lease runs out. Every adapter poll
    renews it. Losing input is worse than losing a highlight, so this lease is
    the safety rule for the whole phase.
- **New query** `Query::InspectPick {}` returns `{ mode, generation,
  hovered: Option<id>, picked: Option<id>, pick_sequence }`. The pick is read
  once per `pick_sequence`, so a repeated poll never reports the same click
  twice.
- Record the new tags in `Query::names()` and the command name list, and
  advertise the support in `RuntimeInfo` so that clients can check before
  using it. A headless runtime reports the feature as unavailable.

### Adapter (`rake-cli/src/cdp/dom.rs`)

| CDP | Rake |
| --- | --- |
| `Overlay.highlightNode { nodeId \| backendNodeId }` | `HighlightElement { id, generation }` |
| `Overlay.hideHighlight` | `HighlightElement { id: None }` |
| `Overlay.setInspectMode { mode: "searchForNode" }` | `InspectMode { enabled: true }`, then poll `InspectPick` about every 100 ms and renew the lease |
| `Overlay.setInspectMode { mode: "none" }` | `InspectMode { enabled: false }` |
| hover in pick | `Overlay.nodeHighlightRequested { nodeId }` |
| click in pick | `Overlay.inspectNodeRequested { backendNodeId }`; the frontend then calls `DOM.pushNodesByBackendIdsToFrontend`, which must be implemented |

- Keep `backendNodeId` stable within a capture, and use the same integer map
  as `nodeId`.
- If the generation reported by `InspectPick` doesn't match the DOM the
  frontend holds, send `DOM.documentUpdated`, rebuild the maps and then
  report the pick.
- When the WebSocket closes, send `InspectMode { enabled: false }` and clear
  the highlight right away. The leases cover a crashed adapter.
- Debounce identical consecutive hover reports. Highlight changes only when
  the hovered ID changes.

### Shared consumers

The same commands give hover-to-highlight and click-to-pick to the HTML
inspector's Elements panel (`scripts/inspector.js`) and to `examples/inspector`.
Wire them up after the CDP path works. A separate commit for each is fine.

### Known limits (document in `docs/element-inspection.md`)

- Z-order is the order elements are laid out in, so deferred or absolutely
  positioned elements may be picked wrongly.
- Only instrumented boundaries can be picked, and the 256-node capture cap
  still applies.
- Polling adds about 100–250 ms of hover lag, more through the Web broker.
  A push channel is a later change.

### Tests

- Runtime: hit-testing (nesting, clipping, uninstrumented gaps, empty capture);
  lease expiry restores normal input; Escape cancels; stale generation is
  rejected; each pick is read once.
- Native integration: synthetic mouse events in inspect mode don't reach an
  app handler, and do reach it after the lease runs out.
- Adapter: the CDP → command table; documentUpdated on a generation mismatch;
  disconnect clears the mode.
- CLI parity: `rake highlight ID|none` and `rake inspect pick` terminal paths,
  following `every_advertised_wire_tag_has_a_raw_terminal_path`.

**Gate:** in the routing example, under both desktop and Web, the Chrome
DevTools picker selects the hovered element in the Elements tree. Hovering
tree nodes highlights them in the app. Closing DevTools during a pick returns
normal input within one lease.

## Phase 6: Console evaluation (needs owner approval)

`Runtime.evaluate` with `name(json)` syntax → `Command::Run` for a registered
typed command. Each evaluation is one mutation with a fresh retry key. An
uncertain delivery returns an error that names the key and is **never** resent
automatically. `Runtime.compileScript` and autocomplete
(`Runtime.globalLexicalScopeNames`) come from `Query::Commands`. This is the
first CDP write path, so it needs the owner's approval first.

## Integration and docs (alongside each phase)

- `rake dev` prints the CDP URL next to the inspector URL when `--cdp` is
  passed. The HTML inspector gets an "Open in Chrome DevTools" link.
- `docs/devtools.md` gets a "Chrome DevTools" section: what each domain shows,
  what is unavailable (headers, bodies, JS debugger, CPU profiler) and the
  security model.
- Checks: `cargo fmt --check`, `cargo clippy -p rake-cli --all-targets`,
  `cargo test -p rake-cli`, plus `cargo test -p rake` if `RuntimeInfo` changes.
  Run a Wasm build check if any `rake` change touches shared types.

## Out of scope

- Forking or bundling `devtools-frontend`, or shipping Chrome extension panels.
- JS/Rust debugging (`Debugger.*`), CPU or heap profiling, and screencast.
- Capturing headers or bodies. This would change Rake's privacy rules, so it
  needs a separate decision.
- Replacing Rake's own inspector panels with CDP.
- Non-Unix native IPC (inherited limit).

## Deliverable

- `.agents/reports/cdp-adapter-spike.md` (Phase 0 findings and the gate result).
- Then one branch per phase (or a stacked series), each with tests and the docs
  update. Report the checks you ran, the Chrome version used and every CDP
  method that still returns an error.
