# macOS native preview prototype

```sh
cargo build -p rake-app-cli --offline
target/debug/rake dev -p rake-preview --preview
```

`--preview` opens a local browser page while the real native app keeps running. It needs macOS 14+, Xcode command-line tools (`xcrun swiftc`) and an app using `ControlConfig::FromEnvironment`. The supervisor enables `rake/devtools`, and `--no-open`, `--port`, `--json` and `--cdp` can be combined with the flag.

The standalone fixture has no router reload checkpoints, so source rebuilds can stay `reload_blocked` while the app stays alive. Restart `rake dev` after editing it. Apps with router reload support keep their existing behavior.

Ordinary development/build/bundle behavior is unchanged. Web and
non-development preview requests are rejected.

The example has a native blurred background, a click counter, an opaque/blurred
backdrop toggle, and forty scrollable rows. Other applications can use
`rake dev -p APPLICATION --preview` with their existing inspected root or routes.

## Capture and input

The supervisor compiles a bundled Swift helper into its private session directory. ScreenCaptureKit captures the first inspected window, verified against both its native window ID and the supervised app's PID. This is native window capture, not browser rendering of markup or an offscreen test host.

The system may request Screen Recording access for the launching terminal or app. If denied, the preview shows the helper's error and restart guidance.

The primary path is ScreenCaptureKit → hardware VideoToolbox H.264 (AVCC) → a binary HTTP response → browser WebCodecs → canvas. This follows the approach of [serve-sim](https://github.com/EvanBacon/serve-sim) with Rake's own framing and metadata, and isn't wire-compatible with it. Frames are never base64 encoded or polled as JSON.

Capture targets 60 FPS within a 1920 × 1440 pixel box (up to 2× native points). A browser or encoder failure falls back to a binary JPEG stream at 30 FPS. Audio is disabled.

Packets have two big-endian u32 lengths (JSON metadata, binary payload), followed by metadata and an AVCC configuration, H.264 access unit, JPEG or idle heartbeat.

- Each encoded payload is bounded to 2 MiB.
- The server keeps at most 120 packets and 8 MiB per codec, with a small send buffer and a write deadline.
- Slow viewers skip to a fresh IDR, and an idle encoder can produce one on request.
- Browser decode queues are bounded, and only the newest frame is painted per animation frame.

Displayed FPS measures canvas presentation, including repeated images, not distinct GPUI renders. App inactivity, GPU load, thermal pressure, browser visibility and native rendering also affect it.

Input and metadata use a separate authenticated WebSocket, so video continues while input is acknowledged.

- Hover is coalesced, and wheel events accumulate per animation frame.
- Fractional motion is kept to 1/64 point, and scroll phases reach GPUI with no synthetic momentum.
- Outstanding input is bounded, and clicks are never retried.
- Encoders stop after viewers disconnect. Resizing or replacement changes a viewport epoch and restarts capture.
- Input stays disabled until video and control agree on the viewport.

If another window fully covers the app, preview input requests a native layer redraw, because GPUI's display link pauses while occluded. Stopping the supervisor ends its helpers. Keep the real window open on the active desktop.

**Interact** forwards hover, primary clicks and wheel deltas through the authenticated runtime protocol. Clicks dispatch movement, down and up atomically on the UI loop, once with no retries. Forwarding doesn't move the desktop cursor or need Accessibility permission.

Keyboard, drag, native title-bar controls, menus and separate system dialogs aren't covered.

Preview windows opt out of Stage Manager's layout while the application is
running in preview mode; otherwise macOS captures an inactive window's rotated
sidebar thumbnail. This changes only that window's collection behavior, not the
system preference. **Reconnect capture** restarts the helper after a session or
permission change.

## Annotation workspace

The native window is centered on black and fits the available viewport. A floating,
borderless toolbar toggles **Annotate** (`C`); clicking it again, `V` or Escape
returns to normal application interaction. The
compact top-right HUD shows live connection state, displayed FPS and reconnect;
its tooltip retains codec, dimensions and native backdrop details.

In Annotate mode, click an element to write a note. The live picker outlines the deepest inspected element without activating it, and automatic text boundaries resolve to their containing paragraph, button or other element. Picking opaque or cached content resolves to an inspected ancestor.

Notes keep the Rust construction site, semantic and stable identities, text, macro-authored style metadata and bounded source ancestry from the exact capture.

Enter saves, Shift+Enter adds a line, and Escape cancels and returns to Interact. Scrolling works in either mode.

Hover or focus the toolbar's comments badge to see saved annotations. Hover or focus a note to outline its element. Clicking a note or pin edits it, and the remove button deletes it.

Clicking the badge copies all comments as plain text, each above its bracketed reference. After a successful copy, a small popover offers **Keep** and **Clear**:

- **Clear** removes the copied notes, keeping any added or edited after the copy.
- **Keep**, Escape or clicking outside dismisses it without clearing.

Failed copies leave all notes intact, and an empty badge can't copy. `Cmd/Ctrl+C` on a hovered note copies just that note. Notes are never sent to an agent automatically.

The picker follows [React Grab's copy workflow](https://github.com/aidenybai/react-grab#readme): hover an element and press `Cmd/Ctrl+C` to copy its source reference, including from Interact mode. In Annotate mode, a cursor-following label shows the element, Rust site and shortcuts. Bare `C` copies the hovered target, and Enter or a click opens the comment editor. Normal copying of selected text stays with the browser.

A copied annotation looks like this (the exact tags, text and source sites come
from the captured tree):

```text
Make this action more prominent.
[<button id="save">Save</button> in SaveButton (at src/button.rs:42:9) in Editor (at src/editor.rs:18:5)]
```

The format matches React Grab's comment-plus-reference convention. The inline preview uses Rake's view tag or type, semantic ID, literal authored classes and normalized descendant text.

- Text is bounded to 100 escaped characters, classes to 15 and IDs to 120.
- At most three distinct source frames appear. These are observed Rust inspection boundaries, not React owners or a fabricated call stack.
- Opaque or missing parents stay unavailable, and older notes fall back to their captured site.
- Full styles and runtime and capture IDs are omitted.

The preview is a diagnostic representation, not HTML rendered by GPUI.

Hover reads are coalesced to at most ten per second, with one outstanding, on a separate authenticated HTTP request, and they omit styles. Source ancestry and descendant text are collected only for explicit copy or comment gestures, using bounded exact-generation pages, and a stale or partial capture keeps only observed context. Pointer exit, mode changes, resizing and disconnection cancel hover reads.

Up to 32 notes, each with 2000 characters, are stored in this tab's browser session,
scoped to the runtime and native host. Reloading restores them; a runtime/window
replacement starts a separate set. Storage errors keep notes usable in memory and
prompt copying them before closing. There is no repository/source modification,
server persistence or screenshot storage.

Live hover previews resolve stable identities against bounded, pinned tree pages. Ambiguous targets match only their original capture, and removed, hidden or clipped targets never highlight a different element by name. Old positions are hidden until refreshed.

Geometry lookups run only while annotation UI is active, at most twice per second, on a separate HTTP request so they can't delay clicks or scrolling. They copy only IDs and geometry, with page, count and deadline bounds.

## Coordinates and lifetime

AppKit supplies the content rectangle inside the window, including the title-bar offset, and canvas coordinates convert to GPUI content logical points.

- Every input binds to the runtime, host and native window ID.
- Live input also binds to a viewport fingerprint, so scrolling survives tree updates and resizing rejects the old viewport.
- Selection pins the exact inspection generation, and references expire after five seconds.
- Non-finite or out-of-bounds positions and wheel deltas over 2000 points per event are rejected.
- Clicks never retry automatically.

Native video and inspection snapshots are sampled independently, with no exact GPU-frame-to-tree correlation. Animated views may reject selections until they settle.

Native materials need checking per OS and capture mode. A nonopaque window flag doesn't prove blur was captured, and desktop-independent capture may substitute a flat backdrop. The fixture's opaque and blur toggle checks native state separately from the video. Full desktop compositing isn't promised.

## Development boundary and checks

The preview shares the server's loopback binding, random token, Host and Origin validation and response bounds. The WebSocket needs a same-origin handshake, with the token in a subprotocol and never a query string. Pixel data and input require authentication.

Runtime bindings also need `RAKE_DEV_PREVIEW=1`, set by the supervisor. Without `rake/devtools`, native bindings and input are compiled out, and production builds keep no preview window state or capture helper.

```sh
python3 scripts/test-preview.py
node scripts/test-preview-ui.mjs
python3 scripts/test-dev-bridge.py
cargo test -p rake-app-cli --bin rake --offline
cargo test -p rake-app --no-default-features --features devtools --lib native_preview --offline
cargo test -p rake-app --test inspection_compiled_out --offline
```

The script suites run mapped Rust contract tests and real HTTP assertions for routing, bounded input, stale references, source metadata and
helper cleanup using explicit doubles. They do not establish native GPU/capture
or browser behavior.

Native verification (macOS GUI, compiled fixture):

```sh
cargo build -p rake-preview --features devtools --offline
python3 scripts/test-preview-native.py
python3 scripts/test-preview-native.py --occluded
```

The native regression exercises real click state, scrolling, source picking,
blur/opaque state and stale-reference rejection. The occluded variant launches a
separate cover process, verifies WindowServer reports full occlusion, then checks
that clicks and scrolling update the inspected presentation in both directions. The browser must additionally
be checked for decoding, viewport mapping, interactions and reconnect behavior.

## Inspection performance regression check

`python3 scripts/test-preview-native.py --benchmark` runs the native fixture checks, warms eight small scroll inputs, then records draw durations while submitting 25 more. It reports the first retained page (up to 20 draws) with median, p95 and maximum wall time. Recording is in memory, and a binary path argument compares preserved builds without changing sources.

On one development Mac, the unoptimized debug fixture measured 52.4 ms median draw time before capture optimization and 12.7 ms after (20 draws each). This measures native draw wall time, not unique video FPS, GPU completion or a universal guarantee. Release builds and other trees need their own measurements, and the benchmark doesn't need ScreenCaptureKit.
