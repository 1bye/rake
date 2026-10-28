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
