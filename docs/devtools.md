# Chrome DevTools

Stock Chrome DevTools is the supported graphical frontend. Rake's older HTML and
`rake-inspector` frontends are legacy prototypes; their controls and tests do not
establish Chrome compatibility. New development targets the CDP adapter.

Build the CLI with `cargo build -p rake-cli`, then run:

```sh
target/debug/rake dev --target desktop --package rake-view --cdp --no-open
target/debug/rake cdp --session /tmp/rake-dev-EXAMPLE --no-open --json
```

The first command builds and runs the example with a CDP adapter; the second
attaches an adapter to an existing session. Copy its printed `devtoolsFrontendUrl`
into stock Chrome. The dev supervisor still starts the legacy inspector service;
use the CDP URL for supported workflows. Python 3.11+ is required by the supervisor.

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

Without `rake/devtools`, automatic `view!` inspection wrappers, source descriptors,
capture/edit/recording state, standalone inspection hosts and inspection wakeups
are compiled out on native and WASM. Native text accessibility, focus, scrolling
and ordinary GPUI styles remain available. Inspection queries report unavailable;
runtime metadata cannot activate code that was compiled out. The protocol types
and unavailable-query responses remain for compatibility.

Manual `inspect` builders return their original element unchanged in that build.
Use `.inspect_with(|| format!("row-{id}"))` for dynamic names, so Rust does not
evaluate inspection-only allocations in production. Ordinary method arguments
are always evaluated by Rust, including arguments to no-op methods. Automatic
macro expansion removes the metadata expressions entirely. With inspection off,
`view!` returns the original component/element types; `into_inner()` is needed only
for wrappers in feature-enabled builds.

Cargo features are additive across dependencies. Production builds must omit
`devtools` throughout the graph; `--release` alone does not disable an explicitly
enabled feature. Inspect the graph with `cargo tree -e features -i rake`.

`rake cdp --session /tmp/rake-dev-EXAMPLE --no-open` starts a separate Rust
adapter for opted-in runtimes. It prints the actual OS-assigned port and stock
`devtools://devtools/bundled/devtools_app.html?ws=…` URLs. Paste a URL into Chrome,
or add the printed loopback address in `chrome://inspect`. `--instance` filters by
runtime UUID or application ID; `/json/list` advertises one page target per live
matching runtime. `--port`, `--no-open` and `--json` are supported.

The adapter binds IPv4 loopback, validates the exact bound Host, accepts only an
absent Origin or `devtools://devtools`, and authenticates WebSockets with a random
session path. It limits connections, message sizes, event retention and I/O time.
Treat discovery URLs as private development credentials. Sessions detach when a
runtime disappears or changes identity. Unsupported CDP methods return `-32601`.
Standalone `App::root` hosts support the same Elements, Console diagnostic, Network
and trace inspection paths when explicitly enabled with `ControlConfig::FromEnvironment`.
They advertise no routing/application commands; typed command completion is empty.
The `rake-view`
example opts into this development policy, so `rake dev --target desktop --package
rake-view --cdp` can inspect its automatic macro-authored tree.

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

Console shows structured tracing events through Log and Runtime, with inline
previews and read-only JSON property expansion. Object retention is limited to
128 structured objects and 1 MiB per connection, plus 128 bounded navigation node
handles; evicted IDs return an error. Loss warnings
report collector drops, output drops, sequence gaps and outbound queue overflow.
With `--session`, supervisor process text appears as `[stdout]`/`[stderr]` using
its own observation clock and without inferred runtime ownership. Structured
events can also appear in process stderr; these are distinct sources.

`rake cdp --session PATH --export-trace trace.json` exports retained spans as
Chrome trace-event JSON for Perfetto or Performance's Load profile. Synchronous
nesting uses complete events, overlapping unrelated spans use async begin/end
IDs, and incomplete captures use flagged instant events. The export is bounded
to 4 MiB and includes loss/omission counts. Times describe elapsed wall time,
with a logical thread lane; they are not CPU samples. Live `Tracing.start`/`end` record GPUI frames and source-attributed inspected callback
wall timings. Both ReportEvents and ReturnAsStream (uncompressed JSON, IO.read/close)
are supported, with a 4 MiB adapter budget and explicit loss reporting. This does not
record browser rendering, V8 CPU profiles or GPU execution. See frame recording below.

`rake dev --target desktop --package rake-routing --cdp` starts the adapter under
the supervisor and prints its discovery URL beside the HTML inspector URL. The
HTML inspector offers **Open in Chrome DevTools** for its selected target. Chrome
may require copying that link into its address bar. A later `rake inspect --session
PATH` can reuse the same running adapter.

Elements projects captured boundaries, including automatic `rake::view!` nodes,
into a synthetic document, with semantic IDs,
authored styles and measured width/height. Source locations stay in Rake inspection
details rather than repeated `data-source` attributes in Chrome's tree. Omitted
boundaries appear as a comment. Optional runtime presentation identities keep CDP
node/backend IDs stable across captures. Geometry changes refresh styles and box
values; structural and attribute changes send targeted DOM events, preserving
unaffected selection and expansion. Reordered edges use remove/insert events with
the moved subtree's delivered expansion. Removal expires an identity; reappearance
does not reuse its old CDP node ID. Ambiguous repeated construction sites need
distinct GPUI IDs to remain stable. A full document refresh is reserved for runtime
replacement, availability changes, legacy captures without identities, or a mutation
batch that cannot fit atomically in the bounded outbound queue. Tree expansion loads
only the requested levels. Picking delivers missing ancestor paths without replacing nodes Chrome
already knows, so expansion and selection do not require enabling the picker.
Solid sRGB colors, typography, borders, spacing, overflow and layout refinements
are translated to CSS values, with captured ancestor declarations shown as inherited
styles. Unsupported values remain visible under `--gpui-` metadata properties.
Chrome's box model uses distinct margin, border, padding and content quads in
window logical pixels, before clipping. Border bounds are measured by GPUI; absolute
pixel/rem edges use the pinned GPUI engine's device-pixel and border-stroke rounding,
from the style sampled during request-layout. Content masks remain separate.
GPUI's public API does not expose used auto/percentage edges or scrollbar metrics:
those boxes are explicitly unavailable. `DOM.getBoxModel` returns an error when any
required box is unknown, including on older runtimes and opaque boundaries, instead
of duplicating the border quad.

Computed exposes the live resolved GPUI snapshot, measured width/height and pixel
rem lengths. Styles separates read-only synthetic rules labelled **GPUI defaults**,
**Inherited text context**, **Authored base**, **DevTools base override**, **Active interaction change**, and
**Measured layout**. The authored base keeps its original declarations, so overridden
values can be inspected alongside the active result. These are observed GPUI origins,
not a browser CSS cascade: inherited text can cross uninspected ancestors, and the
public resolver cannot identify the specific hover/focus/active/drag rule or a
same-valued override. Source opening uses the explicit Console helper below.
Macro-authored
style attributes additionally carry their own locations and expression spellings:
Chrome's node properties expose the read-only `rakeSource`, `rakeStyleSources` and
`rakeHandlers` JSON strings. These declarations are not a claim about which
conditional branch or private GPUI interaction rule supplied the live value.
Legacy snapshots show authored styles without
inventing provenance. Unsupported values remain `--gpui-` metadata properties.
A read-only virtual style snapshot sends CSS change notifications, so hovering the
app updates selected values without rebuilding the document. Force-pseudo-state
controls remain unavailable; hover reflects the real app pointer.
Highlighting and inspect mode use the leased development inspection controls
described in [element inspection](element-inspection.md). Chrome tree hover
highlights the app; its picker selects the clicked instrumented boundary.

Chrome's normal disclosure arrows and **Expand recursively** load captured nodes.
Cmd/Ctrl+F searches names, semantic IDs (`#increment`) and text, revealing collapsed
ancestors. Context actions **Scroll into view**, **Focus**, **Copy outerHTML** and
**Copy selector** operate on the captured tree. **Accessibility** shows captured
names, roles and states; its **Show accessibility tree** switch retains selection.

Add declarations under **DevTools base overrides** in Styles and use Chrome's
declaration checkboxes to toggle them. Acknowledged declarations are available
immediately; resolved values and geometry update after a real GPUI draw. Unchanged
lightweight tree polls do not invalidate the Styles editor. The Console helpers
`rake.resetStyles($0)` and `rake.undoStyles($0)` reset/undo the selected boundary's
temporary overrides. Chrome DOM undo/redo,
arbitrary new CSS rules and CSS selector editing remain unavailable.

Temporary text editing uses Chrome's normal inline text editor: double-click a
text value in Elements, enter its replacement and press Enter. Supported captured
GPUI text and composed paragraph text advertise `text_editable`; inputs and opaque
custom text remain read-only. The replacement is literal text, including Unicode,
newlines and empty strings. GPUI recalculates wrapping, bounds and accessibility
after the real draw; stable node identities and application state are preserved.

For the selected text node, or a container containing only that text boundary, type
`rake.resetText($0)` or `rake.undoText($0)` in Chrome Console. Reset reveals the
application's current content; undo restores the preceding override, including a
reset. These are separate from Chrome's browser DOM undo/redo. CDP clients can use
`DOM.setNodeValue { nodeId, value }`, `Rake.resetText { nodeId }` and
`Rake.undoText { nodeId }`. Replacements use the paragraph's base text style;
reset restores the application’s current inline runs. Rust source and editable field values
are never changed. Histories retain up to 32 edits across at most 64 text nodes,
with a 16 KiB UTF-8 replacement limit and a shared 4 MiB text-history budget.
Removal/window closure drops overrides; process restart clears them. This entire
backend remains behind the opt-in `devtools` feature.

Sessions wait up to five seconds for a unique successor whose `restored_from`
matches the selected RuntimeId, including when `--instance` originally named that
UUID. Replacement clears runtime contexts, cursors, objects and DOM IDs. An
unrelated instance with the same application ID is never followed.

Console evaluation accepts only a registered typed command with JSON arguments,
for example `demo.echo({"message":"Hello from Chrome"})` or `demo.fetch({"url":
"http://127.0.0.1:8081/"})`. Each evaluation submits one `Command::Run` with a
fresh retry key. The result contains that key and the operation's state/result;
queued, running and awaiting-decision states are not a successful commit. An
uncertain delivery error names the key and is never resent automatically. Use
`rake ctl` to observe or explicitly retry with the identical payload and key;
evaluating again submits a new mutation. Eager/side-effect-free evaluation is
rejected. Registry names and non-persisting compile checks read `Query::Commands`.
`rake.help()` lists Console helpers, `rake.commands()` returns application command
schemas, and `rake.command("demo.echo")` describes one command. Namespaces such as
`rake.` and `demo.` provide read-only completion data. Results label their operation
state and include an `observe` token: pass that literal JSON to `rake.operation(...)`
to observe without resubmitting. Tokens include the runtime UUID and expire on
replacement. General JavaScript evaluation, persisted scripts and autocomplete
that executes arbitrary JavaScript remain unavailable. `Runtime.callFunctionOn` recognizes only Chrome's zero-argument
focus/scroll helpers on retained node objects; it never evaluates JavaScript.

Elements Find (`Cmd/Ctrl+F`) searches captured names, IDs and text, including collapsed
branches; `#id` matches an exact semantic ID. Its context menu can focus registered
elements and scroll through registered containers. Both Rake frontends expose the
same actions plus **Open Rust source**. In Chrome's Console, use
`rake.openSource($0)` for the selected Elements node, or
`rake.openSource("routing.command-echo")` (or a retained numeric CDP node ID) to
open the construction site using the system default editor. This explicit host
action is restricted to existing Rust files inside `RAKE_SOURCE_ROOT` (default:
the adapter host's current directory). The editor helper returns no source contents; captured diagnostic source links can
open bounded workspace Rust files as read-only text through authenticated loopback URLs.
line/column remain metadata when the default editor cannot accept a location.
For an attribute or registered handler site, use
`rake.openSource($0, {"kind":"style","index":0})` or
`rake.openSource($0, {"kind":"handler","index":0})`. Indices refer to the
captured `rakeStyleSources`/`rakeHandlers` arrays. `$0` alone returns a retained
node object with read-only source, style and accessibility metadata; it follows
stable captures and expires on removal. These exact helpers are adapter commands;
general JavaScript expressions are unavailable. Keep Chrome's Console paste
protection enabled and type reviewed commands manually when Chrome blocks pasting.
See [navigation and handle registration](element-inspection.md#elements-navigation).

Supported CDP methods are the domain enable/disable methods for Network, Log,
Runtime, DOM, CSS and Overlay; Inspector.enable; Page.enable/disable,
Page.getResourceTree/getFrameTree/bringToFront; Runtime.runIfWaitingForDebugger,
Runtime.evaluate, Runtime.compileScript (without persistence),
Runtime.globalLexicalScopeNames, Runtime.getProperties,
Runtime.releaseObject/releaseObjectGroup; DOM.getDocument, DOM.requestChildNodes,
DOM.describeNode, DOM.getAttributes, DOM.getOuterHTML, DOM.getBoxModel, DOM.performSearch,
DOM.getSearchResults/discardSearchResults, DOM.focus/scrollIntoViewIfNeeded,
DOM.setNodeValue (supported captured text only),
DOM.resolveNode/requestNode/setInspectedNode, Runtime.callFunctionOn (navigation only),
DOM.pushNodesByBackendIdsToFrontend; CSS.getMatchedStylesForNode,
CSS.getInlineStylesForNode, CSS.getComputedStyleForNode; Overlay.hideHighlight.
Network.getResponseBody/getRequestPostData return descriptive unavailable errors.
Overlay.highlightNode/setInspectMode require advertised runtime interaction support. Every unlisted method returns `-32601`,
including Debugger, Profiler, structural DOM edits and other JavaScript function calls.
CSS edits are restricted to temporary DevTools base override sheets; authored styles remain read-only.
Accessibility.enable/disable, getRootAXNode/getFullAXTree/getPartialAXTree/
getChildAXNodes/getAXNodeAndAncestors, Tracing.start/end and IO.read/close are also
supported. `Rake.openSource { nodeId, target? }` accepts captured construction/style/handler sites.
Chrome Copy element receives escaped
synthetic markup. See the [Elements coverage checklist](devtools-next.md#elements-coverage-checklist)
for verification evidence and remaining GPUI boundaries.

## Legacy inspector prototypes

An [Elements-only Inspector built with Rake](rake-inspector.md) remains in the repository
for native and Web: `rake inspect --elements --target desktop|web`. It uses the
experimental `view!` macro and the same runtime queries. Both this preview and the
multi-panel HTML frontend below are unsupported historical prototypes.

### Historical HTML panels

| Panel | Inspection and actions |
| --- | --- |
| Workspace | Windows with independent selections, visibility and guarded close; tab policies, destinations, full retained histories/cursors, entry revisions and dirtiness; select, Back, Forward, close and change policy; guarded quit with wait/cancel job policies, remaining counts and Cancel quit. |
| Routes | Forms generated from registered typed destination descriptors; open/reveal with an explicit host and reveal scope, or navigate/replace in an exact tab, with independent work policy and activation options. Native targets advertise NewWindow. |
| Tasks | Lifetime owner separately from causal origin; task kind, eligibility, state, RunId and generation; cancel and follow related diagnostics. Explicit application progress is unavailable unless logged. |
| Operations | Queued/running/awaiting-decision/terminal states, retained results and errors; inspect or cancel through the shared dispatcher. Commit does not imply finished data loading. |
| Decisions | Pending guard messages and advertised choices. Responses use the same revision checks as GUI/CLI responses; obsolete choices cannot approve changed state. |
| Console | Structured tracing severity/fields/context, bounded build and native stdout/stderr, registered typed commands with argument/result schemas and JSON input. |
| Requests | Method, redacted URL, status, elapsed time, bytes consumed and observed lifecycle events; distinguish headers, completion, cancellation/failure and abandonment. |
| Traces | Collector-local span identities and parent links, inherited fields, elapsed wall-time timeline and related tasks/requests/logs. Missing ancestry or closes remain explicit. |
| Elements | Presented inspection boundaries, real layout, authored and live resolved styles, source sites, semantic IDs and exact capture lookup; stale selections remain frozen. See [coverage](element-inspection.md). |
| Reload | Supervisor build/reload/error status and the runtime lifecycle boundary, generation and restoration origin. |

Search, context filters, paging and JSON diagnostic export operate on retained
observations. Related links only use actual recorded context. Process text has
its own supervisor elapsed clock and no inferred runtime/tab ownership. Structured
tracing can appear again in process stderr; these remain distinct sources.
Legacy `log` forwarding requires the embedding host's tracing bridge. Arbitrary
browser console output and arbitrary Rust evaluation are not provided. The routing
example registers `demo.echo` and `demo.fetch` for explicit command/log/HTTP checks.

## One authoritative control surface

All runtime reads and actions use protocol-1 `Request`/`Reply` envelopes through
`RuntimeClient`: the same queries, commands and event cursors as the terminal.
The host only proxies private native IPC or the authenticated Web broker. It
does not mutate router state or approve decisions. Browser snapshots come from
Events, never reconstruction from logs. Revision gaps replace state with a fresh
authoritative snapshot and increment the visible resynchronization count.

[Guarded quit](guarded-quit.md) uses the Workspace controls and Decisions panel
through the shared dispatcher. Native exit disconnects its runtime; Web quit
leaves a closed workspace queryable in the same document.

Every mutation has a stable request key. Submission receipt, operation commit and
decision acknowledgement remain distinct. An uncertain delivery offers a retry
of the exact original envelope/key, scoped to its original runtime. A timeout,
disconnect, runtime switch or Pause updates never cancels work or chooses a guard
response. JavaScript forms reject unsafe integer inputs/identities; the typed Rust
client and CLI can represent larger integers.

The UI polls serially, observes bounded diagnostics incrementally, retains at most
1,000 records/2 MiB and renders at most 100 rows per page. It displays runtime
collector/output drops, inspector evictions, missing diagnostic sequences and
state resynchronizations. Capture is best effort: these bounds and transport's
1 MiB frame limit can leave incomplete trees or request lifecycles. Process output
retains 128 records of at most 2 KiB each, with truncation/drop indicators. Nothing
in these buffers defines authoritative task or operation state.

## Capture and access boundaries

Rake HTTP and the native GPUI HTTP adapter are captured. Browser GPUI-internal
Fetch and third-party clients need their own adapters. Authorization/cookie headers,
bodies, URL credentials, queries and fragments are excluded. Request metadata and
causal context remain attached through body completion or abandonment, even if
consumption occurs under another task. DNS/TLS and browser-hidden timings are
unavailable; span durations measure elapsed wall time, not CPU usage.

Control remains opt-in and development-only. The loopback server validates
Host/Origin and requires the private session token. The launch fragment is removed
and stored in that tab's session storage; public assets contain no credentials.
Keep private launch URLs and diagnostic exports private. An inspector-only native
origin serves only inspector assets (including the optional Rake Elements frontend),
validates owner-only discovery/socket paths,
and forwards no arbitrary filesystem or process commands. Web application origins
can inspect Web runtimes only; they cannot proxy native IPC. Production builds
contain no inspector connection metadata.

A pixel/GPU debugger, CPU profiler and native code loader are outside this
stage. Native IPC currently requires Unix. Native replacement continues to use
the external reloader boundary or the explicitly reported checkpointed process
restart described in [development](development.md).

## Live frame recording

CDP `Tracing.start`/`Tracing.end` starts/stops a renewable five-second recording lease.
`record_frames` and `frames { after }` expose the same runtime data to raw control
clients. Closing a host or letting the lease expire releases profiling. Recording
uses GPUI's actual complete Window::draw timings, invalidation counts and optional
dirty-to-draw-end latency. It retains up to 600 frames / 4 MiB and returns bounded,
cursor-based pages with gaps, evictions and continuation metadata.

The slowest 64 inspected callback timings per observed frame include stable
boundary identities and Rust source locations. Their durations are inclusive and
must not be summed across nested boundaries. Request-layout can include component
rendering; it is not a separate measurement of GPUI's private layout solver.
GPU execution, presentation latency and CPU sampling remain unavailable. The GPUI
frame collector has no public upstream loss counter; gaps in its own process-wide
buffer cannot be quantified by Rake. Rake reports its own bounded retention loss.

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
