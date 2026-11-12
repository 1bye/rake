# Chrome DevTools

Stock Chrome DevTools is Rake's graphical frontend, connected through a CDP adapter.
The legacy HTML and GPUI inspectors are disabled and excluded from CLI bundles.
`rake inspect --session PATH` attaches through CDP; `--elements` is deprecated.

AI agents can use the [CDP agent client](cdp-agents.md) for native input, semantic
snapshots, condition waits, screenshots and Rust source lookup.

## Embedded Web DevTools

```sh
cargo build -p rake-app-cli --locked
target/debug/rake dev --target web --package rake-notes --cdp
```

The supervisor opens the application with a small circle in the bottom-right
corner. Click it to open Chrome DevTools beside the application; click it again
to close the panel. The application takes the full viewport while closed. Drag
the divider to resize the panes, or focus it and press Left/Right. The extra pane
headers are omitted. Closing the panel disconnects its inspection session.

Use `--no-open` to print the **Chrome DevTools toggle** URL without opening a
browser. Node 22+ is required for Web `--cdp`. The official Chromium frontend,
pinned by `FRONTEND_REVISION` in the helper, downloads on first open and is cached
under the private session directory. Credentials and runtime data stay on loopback.
The private WebSocket proxy connects only advertised runtimes, with the existing
adapter's Host and Origin checks unchanged. The supervisor owns the companion
process and stops it with the development session. Production bundles contain
neither this shell nor the development button.

For an existing session, run the helper directly:

```sh
node crates/rake-cli/scripts/embedded-devtools.mjs --session /tmp/rake-dev-EXAMPLE --app-url 'PRIVATE_LAUNCH_URL'
```

Open its printed URL. The standalone helper stops with Ctrl-C. This integration
is experimental and uses the same CDP capabilities and limits documented below.
Run `node --test scripts/test-embedded-devtools.mjs` for HTTP/WebSocket boundary
checks; actual frontend behavior also requires Chrome verification.

## Launch Chrome DevTools

Build the CLI with `cargo build -p rake-app-cli`, then run:

```sh
target/debug/rake dev --target desktop --package rake-view --cdp --no-open
target/debug/rake cdp --session /tmp/rake-dev-EXAMPLE --no-open --json
```

The first command builds and runs the example and prints a CDP discovery URL.
Open its `/json/list` endpoint and paste the target’s `devtoolsFrontendUrl` into
Chrome. To attach to a session started without `--cdp`, use the second command
and paste the URL it prints. The supervisor runs in Rust; it requires no Python interpreter.

## Connection

Inspection is an opt-in Cargo feature, disabled by default:

```toml
[features]
devtools = ["rake/devtools"]
```

`rake dev` enables that application feature automatically, or enables the direct
Rake dependency's `devtools` feature (including renamed dependencies). `rake build`,
`rake bundle` and ordinary Cargo builds do not enable it automatically. For manual
development builds, use `cargo build --features devtools`. Runtime capture still
requires `ControlConfig::FromEnvironment` and development connection metadata.

Without `rake/devtools`, automatic `view!` inspection wrappers, source descriptors, capture, edit and recording state, standalone inspection hosts and inspection wakeups are compiled out on native and WASM. Runtime metadata can't activate code that was compiled out.

Native text accessibility, focus, scrolling and ordinary GPUI styles stay available. Inspection queries report unavailable, and protocol types and those responses remain for compatibility.

Manual `inspect` builders return their original element unchanged in that build, and `view!` returns the original component or element types. `into_inner()` is needed only for wrappers in feature-enabled builds.

Use `.inspect_with(|| format!("row-{id}"))` for dynamic names, so production doesn't evaluate inspection-only allocations. Rust always evaluates ordinary method arguments, even for no-op methods, whereas automatic macro expansion removes metadata expressions entirely.

Cargo features are additive across dependencies. Production builds must omit
`devtools` throughout the graph; `--release` alone does not disable an explicitly
enabled feature. Inspect the graph with `cargo tree -e features -i rake`.

`rake cdp --session /tmp/rake-dev-EXAMPLE --no-open` starts a separate Rust adapter for opted-in runtimes. It prints the actual port and stock `devtools://devtools/bundled/devtools_app.html?ws=…` URLs. Paste a URL into Chrome, or add the printed loopback address in `chrome://inspect`.

`--instance` filters by runtime UUID or application ID, and `/json/list` advertises one page target per live matching runtime. `--port`, `--no-open` and `--json` are supported.

**Security.** The adapter binds IPv4 loopback, validates the exact Host, accepts only an absent Origin or `devtools://devtools`, and authenticates WebSockets with a random session path. It limits connections, message sizes, event retention and I/O time. Treat discovery URLs as private development credentials.

Sessions detach when a runtime disappears or changes identity, and unsupported CDP methods return `-32601`.

**Standalone hosts.** `App::root` and `App::root_with` support the same Elements, Console, Network and trace paths when enabled with `ControlConfig::FromEnvironment`. Their `RuntimeClient<NoRoute>` also advertises commands, managed work, guarded quit and updater restart. Navigation, tabs and route-history queries are absent. To try it, `rake dev --target desktop --package rake-view --cdp` inspects the example's macro-authored tree.

Synthetic frame/context metadata supports Chrome's frontend; it represents no
browser DOM, JavaScript debugger, CPU profiler or heap profiler.

Wall times use an adapter observation clock estimated from retained diagnostics;
they may skew by polling/transport latency or the age of the latest record.
Monotonic event timestamps retain the runtime's elapsed microsecond clock. Network
adds a connection offset across replacement so preserved requests share one timeline.

Compatibility evidence and the tested Chrome/protocol revisions are in
[the spike report](../.agents/reports/cdp-adapter-spike.md).

Network shows Rake HTTP Fetch lifecycles, redacted URLs, status, received byte
counts and timings, including cancellation and abandoned bodies. Headers, bodies,
wire byte counts and JavaScript initiator stacks are unavailable. Body/post-data
retrieval returns a descriptive CDP error; reported byte totals are received body
bytes rather than measured compressed network traffic.

Console shows structured tracing events through Log and Runtime, with inline previews and read-only JSON expansion. Object retention is limited to 128 structured objects and 1 MiB per connection, plus 128 navigation node handles, and evicted IDs return an error. Loss warnings report collector drops, output drops, sequence gaps and outbound queue overflow.

With `--session`, supervisor process text appears as `[stdout]` or `[stderr]`, using its own clock and with no inferred runtime ownership. Structured events can also appear in process stderr, and these are distinct sources.

`rake cdp --session PATH --export-trace trace.json` exports retained spans as Chrome trace-event JSON for Perfetto or Performance's Load profile. Synchronous nesting uses complete events, overlapping spans use async begin and end IDs, and incomplete captures use flagged instant events. The export is bounded to 4 MiB and includes loss counts. Times are elapsed wall time on a logical thread lane, not CPU samples.

Live `Tracing.start` and `end` record GPUI frames and source-attributed callback wall timings. ReportEvents and ReturnAsStream (uncompressed JSON, IO.read and close) are both supported, with a 4 MiB budget and explicit loss reporting. This doesn't record browser rendering, V8 CPU profiles or GPU execution. See frame recording below.

Elements shows captured boundaries, including automatic `rake::view!` nodes, as a synthetic document with semantic IDs, authored styles and measured width and height. Omitted boundaries appear as a comment. Source locations stay in Rake's inspection details rather than `data-source` attributes.

**Identity and updates**

- Optional presentation identities keep CDP node and backend IDs stable across captures. Removal expires an identity, and reappearance doesn't reuse its old ID. Repeated construction sites need distinct GPUI IDs to stay stable.
- Geometry changes refresh styles and box values. Structural and attribute changes send targeted DOM events, keeping unaffected selection and expansion. Reordered edges use remove and insert events.
- A full refresh is reserved for runtime replacement, availability changes, legacy captures without identities, or a batch too big for the outbound queue.
- Tree expansion loads only the requested levels. Picking delivers missing ancestor paths without replacing nodes Chrome already knows.

**Styles and box model**

- Solid sRGB colors, typography, borders, spacing, overflow and layout refinements become CSS values, with captured ancestor declarations shown as inherited. Unsupported values appear as `--gpui-` metadata properties.
- Chrome's box model uses distinct margin, border, padding and content quads in window logical pixels, before clipping. Border bounds are measured by GPUI. Absolute pixel and rem edges use GPUI's rounding, from the style sampled during request-layout. Content masks stay separate.
- GPUI's public API doesn't expose used auto or percentage edges, or scrollbar metrics. `DOM.getBoxModel` returns an error when a required box is unknown, instead of duplicating the border quad.

Computed shows the live resolved GPUI snapshot, measured width and height, and pixel rem lengths. Styles separates read-only synthetic rules:

- **GPUI defaults**
- **Inherited text context**
- **Authored base**, which keeps original declarations so overridden values stay visible
- **DevTools base override**
- **Active interaction change**
- **Measured layout**

These are observed GPUI origins, not a browser cascade. Inherited text can cross uninspected ancestors, and the public resolver can't identify the specific hover, focus, active or drag rule, or a same-valued override.

Macro-authored style attributes carry their own locations and expression spellings, exposed as the read-only `rakeSource`, `rakeStyleSources` and `rakeHandlers` node properties. They don't say which conditional branch or private rule supplied the live value. Legacy snapshots show authored styles without provenance, and source opening uses the Console helper below.

A read-only virtual style snapshot sends CSS change notifications, so hovering the app updates selected values without rebuilding the document. Force-pseudo-state controls are unavailable, because hover reflects the real pointer. Highlighting and inspect mode use the leased controls in [element inspection](element-inspection.md): tree hover highlights the app, and the picker selects the clicked instrumented boundary.

Chrome's normal disclosure arrows and **Expand recursively** load captured nodes.
Cmd/Ctrl+F searches names, semantic IDs (`#increment`) and text, revealing collapsed
ancestors. Context actions **Scroll into view**, **Focus**, **Copy outerHTML** and
**Copy selector** operate on the captured tree. **Accessibility** shows captured
names, roles and states; its **Show accessibility tree** switch retains selection.

Add declarations under **DevTools base overrides** in Styles, and use Chrome's checkboxes to toggle them. Acknowledged declarations are available immediately, while resolved values and geometry update after a real GPUI draw. Unchanged tree polls don't invalidate the Styles editor.

`rake.resetStyles($0)` and `rake.undoStyles($0)` in Console reset or undo the selected boundary's overrides. Chrome DOM undo and redo, new CSS rules and selector editing are unavailable.

To edit text temporarily, double-click a text value in Elements, enter its replacement and press Enter. Supported GPUI text and composed paragraph text advertise `text_editable`, while inputs and opaque custom text stay read-only.

The replacement is literal text, including Unicode, newlines and empty strings. GPUI recalculates wrapping, bounds and accessibility after the real draw, and node identities and app state are kept.

For the selected text node, or a container holding only that text boundary, use `rake.resetText($0)` or `rake.undoText($0)` in Console. Reset reveals the app's current content and its inline runs, and undo restores the preceding override. These are separate from Chrome's DOM undo and redo. CDP clients can use `DOM.setNodeValue { nodeId, value }`, `Rake.resetText { nodeId }` and `Rake.undoText { nodeId }`.

- Replacements use the paragraph's base text style.
- Rust source and editable field values never change.
- Histories keep up to 32 edits across 64 text nodes, with a 16 KiB replacement limit and a shared 4 MiB budget.
- Removal or window closure drops overrides, and restarting clears them.

This backend stays behind the opt-in `devtools` feature.

Sessions wait up to five seconds for a unique successor whose `restored_from`
matches the selected RuntimeId, including when `--instance` originally named that
UUID. Replacement clears runtime contexts, cursors, objects and DOM IDs. An
unrelated instance with the same application ID is never followed.

Console evaluation accepts only a registered typed command with JSON arguments, such as `demo.echo({"message":"Hello from Chrome"})` or `demo.fetch({"url": "http://127.0.0.1:8081/"})`. General JavaScript, persisted scripts and autocomplete that executes JavaScript are unavailable. `Runtime.callFunctionOn` recognizes only Chrome's zero-argument focus and scroll helpers on retained nodes.

**Results and retries**

- Each evaluation submits one `Command::Run` with a fresh retry key. The result contains the key and the operation's state or result.
- Queued, running and awaiting-decision states aren't a successful commit.
- An uncertain delivery error names the key and is never resent automatically. Use `rake ctl` to observe or retry with the same payload and key, because evaluating again submits a new mutation.
- Eager, side-effect-free evaluation is rejected. Registry names and non-persisting compile checks read `Query::Commands`.

**Helpers**

- `rake.help()` lists Console helpers.
- `rake.commands()` returns command schemas, and `rake.command("demo.echo")` describes one.
- `rake.` and `demo.` namespaces provide read-only completion data.
- Results include an `observe` token. Pass that literal JSON to `rake.operation(...)` to observe without resubmitting. Tokens include the runtime UUID and expire on replacement.

Elements Find (`Cmd/Ctrl+F`) searches captured names, IDs and text, including collapsed branches, and `#id` matches an exact semantic ID. Its context menu can focus registered elements and scroll registered containers. Use `rake.openSource($0)` in Console to open the selected element’s Rust source. See [navigation and handle registration](element-inspection.md#elements-navigation).

**Opening source from Console**

- `rake.openSource($0)` opens the selected node's construction site in the system default editor. It also accepts `"routing.command-echo"` or a retained numeric CDP node ID.
- `rake.openSource($0, {"kind":"style","index":0})` or `{"kind":"handler","index":0}` opens an attribute or handler site. Indices refer to the `rakeStyleSources` and `rakeHandlers` arrays.
- It's restricted to existing Rust files inside `RAKE_SOURCE_ROOT` (default: the adapter's current directory), and returns no source contents. Line and column stay metadata when the editor can't accept a location.
- Captured diagnostic links can open bounded workspace Rust files as read-only text through authenticated loopback URLs.

`$0` alone returns a retained node object with read-only source, style and accessibility metadata. It follows stable captures and expires on removal. These helpers are adapter commands, not general JavaScript. Keep Chrome's paste protection on, and type reviewed commands by hand when it blocks pasting.

**Supported CDP methods.** Every unlisted method returns `-32601`, including Debugger, Profiler, structural DOM edits and other JavaScript function calls.

| Domain | Methods |
| --- | --- |
| Enable and disable | Network, Log, Runtime, DOM, CSS, Overlay, DOMStorage, Accessibility; `Inspector.enable` |
| Page | `enable`, `disable`, `getResourceTree`, `getFrameTree`, `bringToFront` |
| Runtime | `runIfWaitingForDebugger`, `evaluate`, `compileScript` (no persistence), `globalLexicalScopeNames`, `getProperties`, `releaseObject`, `releaseObjectGroup`, `callFunctionOn` (navigation only) |
| DOM | `getDocument`, `requestChildNodes`, `describeNode`, `getAttributes`, `getOuterHTML`, `getBoxModel`, `performSearch`, `getSearchResults`, `discardSearchResults`, `focus`, `scrollIntoViewIfNeeded`, `setNodeValue` (captured text only), `resolveNode`, `requestNode`, `setInspectedNode`, `pushNodesByBackendIdsToFrontend` |
| CSS | `getMatchedStylesForNode`, `getInlineStylesForNode`, `getComputedStyleForNode`. Edits are limited to temporary DevTools base override sheets, and authored styles stay read-only |
| Overlay | `hideHighlight`. `highlightNode` and `setInspectMode` need advertised runtime interaction support |
| Network | `getResponseBody` and `getRequestPostData` return descriptive unavailable errors |
| Accessibility | `getRootAXNode`, `getFullAXTree`, `getPartialAXTree`, `getChildAXNodes`, `getAXNodeAndAncestors` |
| Tracing and IO | `Tracing.start`, `Tracing.end`, `IO.read`, `IO.close` |
| Storage | `Storage.getStorageKey`, `getStorageKeyForFrame`, `DOMStorage.getDOMStorageItems`, `setDOMStorageItem`, `removeDOMStorageItem`, and change events for registered typed stores |
| Rake | `Rake.openSource { nodeId, target? }` for captured construction, style and handler sites; `Rake.getStorage` for the store catalog |

Bulk clear is rejected and Chrome's optimistic table is restored. Browser cookies, service workers, databases and quotas aren't synthesized. See [typed storage](storage.md) for registration, production feature gates and limits. Chrome's Copy element receives escaped synthetic markup. See [DevTools coverage](internal/devtools-coverage.md#elements-coverage-checklist) for verification evidence and remaining GPUI boundaries.

## One authoritative control surface

All runtime reads and actions use protocol-1 `Request` and `Reply` envelopes through `RuntimeClient`, with the same queries, commands and event cursors as the terminal. The host only proxies private native IPC or the authenticated Web broker, and doesn't mutate router state or approve decisions.

Every mutation has a stable request key. Submission receipt, operation commit and
decision acknowledgement stay distinct. Use [the control CLI](runtime-control.md) to query
operations and respond to guarded decisions.

Process output keeps 128 records of at most 2 KiB each, with truncation
indicators. Diagnostics are observations of the running app.

## Capture and access boundaries

Rake HTTP and the native GPUI HTTP adapter are captured. Browser GPUI-internal Fetch and third-party clients need their own adapters.

- Authorization and cookie headers, bodies, URL credentials, queries and fragments are excluded.
- Request metadata and causal context stay attached through body completion or abandonment, even under another task.
- DNS, TLS and browser-hidden timings are unavailable. Span durations measure elapsed wall time, not CPU usage.

Control stays opt-in and development-only, and production builds contain no inspector connection metadata.

- The loopback server validates Host and Origin and requires the private session token.
- The launch fragment is removed and stored in that tab's session storage, and public assets contain no credentials.
- Keep private launch URLs and diagnostic exports private.
- The development Web bridge handles browser runtimes. The CDP adapter reads private native runtime discovery and socket paths.

A pixel/GPU debugger, CPU profiler and native code loader are outside this
stage. Native IPC currently requires Unix. Native replacement continues to use
the external reloader boundary or the explicitly reported checkpointed process
restart described in [development](development.md).

## Live frame recording

CDP `Tracing.start` and `Tracing.end` start and stop a renewable five-second recording lease. `record_frames` and `frames { after }` expose the same data to raw control clients. Closing a host or letting the lease expire releases profiling.

Recording uses GPUI's actual complete `Window::draw` timings, invalidation counts and optional dirty-to-draw-end latency. It keeps up to 600 frames or 4 MiB, and returns bounded, cursor-based pages with gaps, evictions and continuation metadata.

The slowest 64 inspected callback timings per frame include stable boundary identities and Rust source locations. Durations are inclusive, so don't sum them across nested boundaries. Request-layout can include component rendering, and isn't a separate measurement of GPUI's layout solver.

GPU execution, presentation latency and CPU sampling are unavailable. GPUI's frame collector has no public loss counter, so Rake can't quantify gaps in its buffer, and reports only its own retention loss.

Chrome's Tracing path consumes the same frames; streaming avoids large WebSocket
responses. The exported profile can be read as Chrome trace-event JSON or in
Perfetto. Chrome's browser/JavaScript-specific Performance tracks are not supplied.

Accessibility inspects captured GPUI metadata through Chrome's Accessibility pane;
`rakeAccessibility` and `rakeHandlers` in node Properties retain native focus facts
and Rust binding sites. Complete OS accessibility trees, relationship-derived names, private focus
groups and arbitrary GPUI listener enumeration remain outside current coverage.

Empty frame callback lists indicate unavailable attribution, including cached draws
or evicted pending metadata; omission counts cover known observations only. Pending
callbacks match the actual draw's time interval and cannot attach to a later frame.
Recording adds instrumentation overhead.

Console delivery progress and its verification are tracked in
[the Console checklist](devtools-console.md).

See the [Network checklist](devtools-network.md) for HTTP metadata, bounded body
capture, origins, timing and Chrome workflows.

See the [Performance checklist](devtools-performance.md) for live Record/Stop,
frame and callback analysis, calibrated command/task/span/HTTP tracks, Rust source
links, bounded trace export and stock Chrome verification.

See the [Sources checklist](devtools-sources.md) for captured Rust files,
read-only browsing, search, shared source links and runtime-scoped access.

See the [Memory checklist](devtools-memory.md) for opt-in native allocator
sampling, Rust call sites, sampled live-block comparisons and Chrome export.
It requires `devtools-memory` and an explicitly installed allocator wrapper.
