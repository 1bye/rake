# CDP adapter spike — Phase 0

Gate: **PASS**, 2026-10-28. Chrome 156.0.8078.48 on macOS, bundled frontend
Chromium revision `d0c02bfad08ec92a6ffcf989e53cd995e6381a4c`.
Protocol reference: ChromeDevTools/devtools-protocol commit
`ebca9061ecc4c77b7d6b5b8f7b8afe19dc3ab9ed` (tip observed on test date).

## Evidence

A throwaway Python HTTP/WebSocket server exposed /json/version, /json/list and
page/node targets. An isolated headless Chrome loaded its stock bundled frontend.
The all-32601 baseline opened with no requests/logs. The page target with minimal
replies displayed `spike example.test 200 OK fetch`, `1 requests`, `4 B transferred`
and `Rake CDP spike console proof` in Console, verified in the rendered UI including
shadow DOM. No real browser page backed the fake runtime. Events sent:
Network.requestWillBeSent, responseReceived, loadingFinished and Log.entryAdded.

Successful startup supplied a synthetic frame/resource tree, execution context,
empty DOM document, enable acknowledgements and Runtime.runIfWaitingForDebugger.
Unsupported model methods were tolerated. This is a sufficient contract, not a
proof each reply is necessary. The node/js_app target also opened: this version
exposes Network/Performance/Memory there too, contrary to the older plan assumption.
Use page because it exposes Elements.

macOS `open -a "Google Chrome" devtools://…` exited zero; this alone does not prove
visible navigation. Isolated Chrome definitively opened the bundled URL through
Target.createTarget. Print a pasteable URL and support chrome://inspect discovery.
The pinned hosted frontend returned HTTP 200; its WebSocket compatibility remains
unverified, and production deliberately rejects HTTPS hosted Origin values.
Frame-less live Performance recording remains unverified; implement trace file
export, leaving Tracing.start/end unsupported as the plan permits.

## Real runtime and decisions

Built rake-cli/rake-routing and ran `rake dev --target desktop --package
rake-routing --no-open --json`. Queried real logs and inspect tree through its
private session socket. Ran demo.echo and demo.fetch against loopback HTTP.
Requests have numeric request_id, phase, method, redacted URL, status, per-chunk
bytes, total received_bytes and duration_us. Tree captures have opaque IDs,
parents, real bounds, authored styles and source sites.

The owner's request to implement every phase authorizes the proposed Rust CLI
adapter and typed-command evaluation in Phase 6. No arbitrary JS execution.
Use installed tungstenite 0.28.0, MSRV 1.63 versus project Rust 1.95.0, handshake
only, no TLS/async runtime. Loopback binding, exact Host, absent/devtools://devtools
Origin and random token paths remain required.

Clock decision: estimate Unix epoch from adapter observation time minus newest
retained runtime elapsed time on first diagnostics. Wall timestamps can skew by
poll/transport delay (or stale latest record); this is not a precise runtime start.
Keep protocol and shared runtime types unchanged.

## Methods observed

- `Accessibility.enable`
- `Animation.enable`
- `Audits.enable`
- `Autofill.enable`
- `Autofill.setAddresses`
- `CSS.enable`
- `CSS.takeComputedStyleUpdates`
- `CSS.trackComputedStyleUpdates`
- `DOM.enable`
- `DOM.getDocument`
- `DOM.pushNodesByBackendIdsToFrontend`
- `DOMDebugger.setBreakOnCSPViolation`
- `Debugger.enable`
- `Debugger.setAsyncCallStackDepth`
- `Debugger.setBlackboxPatterns`
- `Debugger.setPauseOnExceptions`
- `Emulation.setEmulatedMedia`
- `Emulation.setEmulatedVisionDeficiency`
- `Emulation.setFocusEmulationEnabled`
- `Inspector.enable`
- `Log.enable`
- `Log.startViolationsReport`
- `Network.configureDurableMessages`
- `Network.emulateNetworkConditionsByRule`
- `Network.enable`
- `Network.overrideNetworkState`
- `Network.setAttachDebugStack`
- `Network.setBlockedURLs`
- `Overlay.enable`
- `Overlay.hideHighlight`
- `Overlay.setShowContainerQueryOverlays`
- `Overlay.setShowFlexOverlays`
- `Overlay.setShowGridOverlays`
- `Overlay.setShowIsolatedElements`
- `Overlay.setShowScrollSnapOverlays`
- `Overlay.setShowViewportSizeOnResize`
- `Page.addScriptToEvaluateOnNewDocument`
- `Page.enable`
- `Page.getResourceTree`
- `Page.setAdBlockingEnabled`
- `Profiler.enable`
- `Runtime.addBinding`
- `Runtime.enable`
- `Runtime.getIsolateId`
- `Runtime.runIfWaitingForDebugger`
- `ServiceWorker.enable`
- `Storage.getStorageKey`
- `Target.setAutoAttach`
- `Target.setDiscoverTargets`
- `Target.setRemoteLocations`

## Phase 5 frontend and integration verification

Chrome 156's stock Elements frontend displayed the routing example's actual
GPUI tree, semantic IDs, source attributes and authored inline styles. Selecting
the workspace node showed display:flex, flex-direction:column and the raw
-gpui-* metadata. Its Computed sidebar displayed measured width/height
(1510px × 964px in the test). Chrome's own metrics widget received those computed
values and all four 1510×964 quads from DOM.getBoxModel. Separate box decorations
are incomplete with width/height-only computed styles; do not fabricate metrics.
The Rake Elements inspector remains the primary source for capture coverage.
New capture IDs can reset stock frontend selection; stable-capture checks disabled
DOM refresh and waited for Chrome's asynchronous node selection to complete.
CSS.getInlineStylesForNode was additionally required by Chrome's metrics model.
Highlight/inspect mode deliberately return a descriptive unsupported error.

A real CLI/WebSocket test restored an empty headless router checkpoint into a new
runtime, kept the old UUID selector, and observed context clearing, documentUpdated
and the new unique context identity. Normal disappearance detaches after a bounded
five-second successor window. The supervisor --cdp smoke run printed both CDP and
HTML inspector endpoints and advertised the new runtime after startup.

Native polling measurement: 20 Info+Diagnostics pairs on the running routing
example had median 0.660 ms and maximum 2.808 ms (Python Unix client including
JSON encoding/decoding). Keep the initial 250 ms interval. This small-record native
measurement does not establish loaded/broker performance.

Checks: cargo test -p rake-cli; cargo clippy -p rake-cli --all-targets;
cargo fmt --all --check; Python inspector (9), supervisor (10), broker (3) tests;
Node inspector model/UI checks; actual Chrome 156 stock frontend. No rake shared
type or runtime implementation changes; no Wasm build was needed for these phases.

## Phase 6 and final verification

The owner's implement-every-phase instruction authorized this typed-command write
path. Evaluation accepts exactly name(JSON), resolves the registered command from
Query::Commands, and submits Command::Run once with a fresh UUID key. Invalid JSON,
unregistered commands, stale contexts and throwOnSideEffect/eager evaluation fail
before dispatch. Compile checks are non-persisting registry/syntax checks. Global
scope names list registered command IDs. Persisted scripts and JavaScript function
calls are deliberately unavailable.

The actual routing example was inspected through stock Chrome 156 after calling
demo.echo and demo.fetch through the implemented Runtime.evaluate path. Network
showed a 200 Fetch with 52 B transferred; Console showed the typed echo and request
completion, structured fields and honest supervisor output loss warnings.

Final Rust checks: cargo fmt --all --check; cargo clippy -p rake-cli --all-targets;
cargo test -p rake-cli (14 unit tests, 3 CDP boundary tests, 1 timeout test and
3 existing runtime tests). The CDP boundary suite exercises real CLI processes,
TCP/WebSockets and native runtime sockets; HTTP capture uses a diagnostics-only
protocol fixture backed by a real HTTP collector. Runtime replacement includes an
unrelated same-application runtime. A lost-reply fixture verified one submission,
no resend and the exact retry key in the error. Outbound events are bounded by
256 entries and 1 MiB per connection; replies above 1 MiB become an error that
preserves an evaluated command's retry key.

Affected script checks: inspector Python tests (9), supervisor tests (10), broker
tests (3), inspector model/UI Node checks, Python compilation and JavaScript syntax.
No shared rake types changed, so no rake/Wasm build was required. No loaded Web
broker performance, hosted frontend WebSocket use, CPU/heap profiling, native
highlight protocol or live Performance recording was established. Trace files are
implemented; live Tracing.start/end remain unsupported under the Phase 0 stretch
gate. Computed bounds and SDK quads are verified; full Chrome box decoration and
CSS inference remain incomplete because only actual captured metrics are returned.

## Error surface

Explicit unavailable: Network.getResponseBody, Network.getRequestPostData,
Overlay.highlightNode, Overlay.setInspectMode, Runtime.compileScript with
persistScript=true, and Runtime.evaluate for JavaScript or side-effect-free/eager
requests. Expired object/node IDs, invalid params and unavailable captures also
return errors. All methods outside the supported list in docs/devtools.md return
-32601. This includes all Debugger, Profiler and Tracing methods, CSS/DOM edits,
Runtime.callFunctionOn and Runtime.runScript.

The following methods observed during Chrome startup/interaction still return
-32601 (unsupported model setup is tolerated by the tested frontend):

- `Accessibility.enable`
- `Animation.enable`
- `Audits.enable`
- `Autofill.enable`
- `Autofill.setAddresses`
- `CSS.takeComputedStyleUpdates`
- `CSS.trackComputedStyleUpdates`
- `DOMDebugger.setBreakOnCSPViolation`
- `Debugger.enable`
- `Debugger.setAsyncCallStackDepth`
- `Debugger.setBlackboxPatterns`
- `Debugger.setPauseOnExceptions`
- `Emulation.setEmulatedMedia`
- `Emulation.setEmulatedVisionDeficiency`
- `Emulation.setFocusEmulationEnabled`
- `Log.startViolationsReport`
- `Network.configureDurableMessages`
- `Network.emulateNetworkConditionsByRule`
- `Network.overrideNetworkState`
- `Network.setAttachDebugStack`
- `Network.setBlockedURLs`
- `Overlay.setShowContainerQueryOverlays`
- `Overlay.setShowFlexOverlays`
- `Overlay.setShowGridOverlays`
- `Overlay.setShowIsolatedElements`
- `Overlay.setShowScrollSnapOverlays`
- `Overlay.setShowViewportSizeOnResize`
- `Page.addScriptToEvaluateOnNewDocument`
- `Page.setAdBlockingEnabled`
- `Profiler.enable`
- `Runtime.addBinding`
- `Runtime.getIsolateId`
- `ServiceWorker.enable`
- `Storage.getStorageKey`
- `Target.setAutoAttach`
- `Target.setDiscoverTargets`
- `Target.setRemoteLocations`
