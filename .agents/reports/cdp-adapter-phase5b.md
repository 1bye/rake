# CDP Phase 5b — leased highlight and picking

Implemented on the existing CDP branch as a separate phase commit.

## Runtime ownership and protocol

The workspace inspection root owns development interaction for each window.
OverlayHost is an optional application component, so it cannot cover every routed
application; the inspection root is always present on opted-in routed hosts.
Captured content-mask intersections and viewport clipping accompany real layout.
Each window retains one bounded capture and one optional highlight. Closed windows
drop their state, timer and Escape interceptor. A background repaint cannot steal
the current window's hovered/picked generation.

HighlightElement uses an exact opaque ID plus capture generation and rejects
StaleGeneration. It paints translucent fill, a one-pixel border and a
kind/semantic-ID/dimensions label. Highlight leases last two seconds. InspectMode
uses a 1–2000 ms lease; clients renew one-second leases about every 100 ms. Capture
listeners suppress moves, clicks, releases and scrolling before app handlers.
Escape cancels before application shortcuts. Input handlers check expiry without
requiring a render; a host-owned timer redraws expired overlays.

InspectPick returns generation, hover, retained pick and pick_sequence. Every click
increments the sequence once, and each client deduplicates it. Identical boundary
metadata/geometry keeps capture IDs stable across overlay-only redraws. Real
changes invalidate them. Control-disabled/headless runtimes advertise interaction
as false. The shared protocol remains version 1 with additive tags/defaulted fields.
These transient development controls retain no operation or retry-key entry;
2048 renewals were tested without consuming application request capacity.

## Consumers

CDP Overlay methods now translate to the exact shared controls. nodeId and
backendNodeId use the same capture-local map. Generation mismatches emit
DOM.documentUpdated before mapped hover/pick events. Backend-node pushes populate
the frontend before their reply. Closing or losing an active session clears mode
and highlight, with leases covering a crashed process. Passive sessions do not
claim cleanup ownership. HTML Elements and the Rake Inspector expose Pick and
hover-to-highlight; their queries/commands use the same protocol. The CLI supports
highlight ID|none, inspect pick, and ctl inspect mode on|off, including --session.

## Actual frontend evidence

Chrome 156.0.8078.48, stock bundled frontend, macOS, guest browser session.

Desktop: the actual routing example advertised element_interaction=true, returned
captured masks and real layout, and accepted the friendly highlight/clear paths.
Chrome's picker selected routing.screen after a click in the actual app. Selecting
its workspace tree row painted the blue translucent overlay and the label
`gpui::elements::div::Div rake.workspace · 1510×964` over the app. Closing DevTools
during a new pick left mode=false in the runtime query.

Web: built the actual routing Wasm library and loader with the matching cached
wasm-bindgen 0.2.129. Through the authenticated broker and CDP server, Chrome's
picker selected routing.edit. The app still displayed Draft edits: 0, proving the
pick click did not run that button. After closing DevTools during another active
pick, the query reported mode=false; a normal click then changed Draft edits to 1.
The disposable Web draft was saved and the guest windows were closed.

Chrome suspended the app when it shared a window with a foreground DevTools tab;
the initial connection closed. Separate app/DevTools windows worked. This known
browser lifecycle limitation is documented. GUI accessibility initially took
several minutes to respond; later actions and screenshots verified real results.
The desktop screenshot directly established highlight painting; a separate Web
hover-highlight screenshot was not retained. Native input tests and the Web pick
exercise the shared capture/paint/input implementation.

## Checks

Passed:
- cargo test -p rake (full native suite and doctests)
- cargo test -p rake --features view-test-support --lib inspection
- cargo test -p rake --no-default-features --lib --test element_inspection
- cargo test -p rake-cli (full CLI and real Unix/TCP/WebSocket tests)
- cargo test -p rake-inspector (library, connection, model and doc tests)
- targeted final inspector connection/model and CLI unit checks
- cargo check -p rake --target wasm32-unknown-unknown
- cargo check -p rake-inspector --lib --target wasm32-unknown-unknown
- actual routing Wasm build with matching loader generation
- cargo clippy -p rake -p rake-cli -p rake-inspector --all-targets -- -D warnings
- cargo fmt --all --check and git diff --check
- Python inspector (9) and development broker (3) checks
- Node inspector model and UI checks, including picker renewal and hover/clear

An initial inspector Wasm check included its native-only binary and failed because
that target has no Wasm main; the supported --lib target passed. The strict broker
fixture required the new defaulted RuntimeInfo capability and passed after update.
Native GPUI TestPlatform dispatch tests establish handler suppression, expiry and
Escape independently of GPU screenshots. Adapter tests exercise command mapping,
generation resynchronization, and cleanup over real sockets.

Known limits remain prepaint-order z-order, instrumented boundaries and the 256-node
cap, polling lag, and browser suspension. Inspection controls are shared runtime
development state; they are not application mutations or a JavaScript evaluator.
