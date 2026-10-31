# macOS native preview prototype

```sh
cargo build -p rake-cli --offline
target/debug/rake dev -p rake-preview --preview
```

`--preview` opens a local browser page while the real native application stays
running. It requires macOS 14+, Xcode command-line tools (`xcrun swiftc`), and
an application using `ControlConfig::FromEnvironment`. The supervisor enables
`rake/devtools`. `--no-open`, `--port`, `--json` and `--cdp` can be combined with
the flag. The standalone fixture does not implement router reload checkpoints:
source rebuilds can remain `reload_blocked` while the current app stays alive.
Restart `rake dev` after editing this fixture. Applications with router reload
support retain their existing reload behavior.

Ordinary development/build/bundle behavior is unchanged. Web and
non-development preview requests are rejected.

The example has a native blurred background, a click counter, an opaque/blurred
backdrop toggle, and forty scrollable rows. Other applications can use
`rake dev -p APPLICATION --preview` with their existing inspected root or routes.

## Capture and input

The supervisor compiles a bundled Swift helper into its private session directory.
ScreenCaptureKit captures the first inspected window, verified against both its
native window ID and the supervised application's PID. This is native window
capture, not browser rendering of markup or an offscreen test host. The system
may request Screen Recording access for the launching terminal/app. If denied,
the preview displays the helper's error and restart guidance.

The primary path is ScreenCaptureKit → hardware VideoToolbox H.264 (AVCC) →
a binary HTTP response → browser WebCodecs → canvas. This follows the transport
approach used by [serve-sim](https://github.com/EvanBacon/serve-sim), with Rake's
own framing and window metadata; it is not wire-compatible with serve-sim.
No frames are base64 encoded or polled as JSON. Capture targets 60 FPS within a
1920 × 1440 pixel bounding box (up to 2× native points). Browser/encoder failure
falls back to a binary JPEG stream targeting 30 FPS. Audio is disabled.

Packets have two big-endian u32 lengths (JSON metadata, binary payload), followed
by metadata and an AVCC configuration, H.264 access unit, JPEG, or idle heartbeat.
Each encoded payload is bounded to 2 MiB. The server retains at most 120 packets
and 8 MiB per codec, with a small socket send buffer and a write deadline. Slow
viewers skip to a fresh IDR; an idle encoder can produce one on request. Browser
decode queues are bounded and only the newest decoded frame is painted per
animation frame. Displayed FPS measures canvas presentation, including repeated
native images; it does not measure distinct GPUI renders. App inactivity, GPU
load, thermal pressure, browser visibility and native rendering still affect FPS.

Input and metadata use a separate authenticated WebSocket. Video continues while
input is acknowledged. Hover is coalesced, wheel events accumulate per browser
animation frame, fractional motion is retained to 1/64 point, and scroll gesture
phases reach GPUI without synthetic browser-side momentum. Outstanding input is
bounded and clicks are never retried. Encoders stop after viewers disconnect;
resizing or replacement changes a viewport epoch and restarts capture. Input stays
disabled until video and control agree on the viewport. If another window fully
covers the native app, preview input explicitly requests a native layer redraw:
GPUI's normal display link pauses while occluded. This runs after the current
GPUI update to avoid re-entering a borrowed application context. Stopping the supervisor
terminates its helpers. Keep the real native window open on the active desktop.

**Interact** forwards hover, primary clicks and wheel deltas through the
authenticated runtime protocol. Clicks dispatch movement, down and up atomically
on the UI loop. They are submitted once without automatic retries. Forwarding
does not move the desktop cursor or require Accessibility permission. Keyboard,
drag, native title-bar controls, menus and separate system dialogs are outside
the prototype's input coverage.

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

In Annotate mode, click an element to write a note in the contextual editor.
The live picker outlines the deepest inspected element without activating the control;
automatic text boundaries resolve to their containing paragraph, button or other element.
Notes retain the Rust construction site, semantic/stable identities, text and
macro-authored style source metadata, plus bounded source ancestry from the exact selected capture.
The compact floating editor grows with the comment. Enter saves, Shift+Enter adds
a new line, and Escape cancels and returns to Interact. Scrolling works in either mode. Picking opaque/cached content
resolves to an inspected ancestor.

Hover or focus the toolbar's comments badge to see saved annotations. Hover or
focus a note to outline its element. Clicking a note/pin edits it; the remove
button deletes it. Clicking the badge copies all comments in the same plain-text
format as a direct element copy, with each comment above its bracketed reference.
Only after copying succeeds, a small popover offers **Keep** and
**Clear**. Clear removes the copied notes; notes added or edited after
the copy are kept. Keep, Escape or clicking outside dismisses the question without
clearing. Failed copies leave all notes intact. An empty badge cannot copy.
Hovering a note also supports `Cmd/Ctrl+C` for copying just that note.
No notes are sent to an agent automatically.

The picker follows [React Grab's copy workflow](https://github.com/aidenybai/react-grab#readme):
hover an element and press `Cmd/Ctrl+C` to copy its source reference, including from
Interact mode. In Annotate mode a cursor-following label shows the element, Rust
site and shortcuts. Bare `C` copies the hovered target; Enter or a click opens the
comment editor. Its leading context icon copies the element without saving a note;
hovering the icon shows the element name and Rust source site.
Normal copying of selected text in the editor/page stays with the browser.

A copied annotation looks like this (the exact tags, text and source sites come
from the captured tree):

```text
Make this action more prominent.
[<button id="save">Save</button> in SaveButton (at src/button.rs:42:9) in Editor (at src/editor.rs:18:5)]
```

The formatter matches React Grab's comment-plus-reference plain-text convention.
The inline preview uses Rake's view tag/type, semantic ID, literal authored classes
and normalized descendant text. Text is bounded to 100 escaped characters; class
previews to 15 and IDs to 120. At most three distinct source frames are displayed.
These are observed Rust inspection boundaries, not React owners or a fabricated
native call stack. Opaque/missing parents stay unavailable; older stored notes
fall back to their captured construction site. The preview is a diagnostic element
representation, not HTML rendered by GPUI. Full styles and runtime/capture IDs
are omitted from the clipboard text.

Hover reads are coalesced to at most ten per second, with one outstanding read,
on a separate authenticated HTTP request. They omit style payloads. Source ancestry
and descendant text are collected only for explicit copy/comment gestures, using
bounded exact-generation pages; a stale/partial capture keeps only observed context.
Pointer exit, mode changes, resized viewports and disconnection cancel hover reads.

Up to 32 notes, each with 2000 characters, are stored in this tab's browser session,
scoped to the runtime and native host. Reloading restores them; a runtime/window
replacement starts a separate set. Storage errors keep notes usable in memory and
prompt copying them before closing. There is no repository/source modification,
server persistence or screenshot storage.

Live hover previews resolve stable identities against bounded, pinned tree pages.
Ambiguous targets only match their original exact capture; removed, hidden or
clipped targets never highlight a different element by name. Old positions are
hidden until refreshed. Geometry lookups run only while annotation UI is active,
at most twice per second, on a separate authenticated HTTP request so they cannot
queue ahead of clicks/scrolling on the input WebSocket. Resolution copies only IDs
and geometry, with page/count/deadline bounds, rather than fetching full styles.

## Coordinates and lifetime

AppKit supplies the content rectangle inside the native window, including the
title-bar offset. Canvas coordinates convert to GPUI content logical points.
Every input binds to the runtime, host and native window ID. Live input also binds
to a viewport fingerprint so scrolling survives ordinary tree updates; resizing
rejects the old viewport. Selection pins the exact inspection generation.
References expire after five seconds. Non-finite/out-of-bounds positions and wheel
deltas over 2000 points per event are rejected. Clicks never automatically retry.

Native video and inspection snapshots are independently sampled; this does not
claim exact GPU-frame-to-tree correlation. Animated views may reject selections
until their presentation settles. Native materials need verification per OS and
capture mode; a nonopaque window flag alone does not prove blur was captured.
Desktop-independent window capture may substitute a flat backdrop for some native
materials. The fixture's opaque/blur toggle checks native state separately from
what the video reproduces; full desktop compositing is not promised.

## Development boundary and checks

The preview shares the existing server's loopback binding, random token,
Host/Origin validation and response bounds. The WebSocket requires a same-origin
handshake with the token offered in a subprotocol (never in a query string). Pixel data and input require
authentication. Runtime bindings additionally require `RAKE_DEV_PREVIEW=1`, set
by the supervisor. Without `rake/devtools`, native bindings/input are compiled
out and production builds retain no preview window state or capture helper.

```sh
python3 scripts/test-preview.py
node scripts/test-preview-ui.mjs
python3 scripts/test-dev-bridge.py
cargo test -p rake-cli --bin rake --offline
cargo test -p rake --no-default-features --features devtools --lib native_preview --offline
cargo test -p rake --features experimental-view --test inspection_compiled_out --offline
```

Python tests cover routing, bounded input, stale references, source metadata and
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

`python3 scripts/test-preview-native.py --benchmark` runs the same native fixture
checks, warms eight small scroll inputs, then records draw durations while submitting
25 more inputs. It reports the first retained page (up to 20 actual GPUI draws),
with median, p95 and maximum wall time. Recording is in memory and stops in `finally`.
A binary path argument allows comparing preserved builds without changing sources.

On the development Mac, the unoptimized debug fixture measured 52.425 ms median /
57.304 ms p95 before capture optimization and 12.676 ms median / 13.101 ms p95 after
it (20 draws each, 2026-10-30). This is about 76% less draw time with inspection and
callback recording enabled. The comparison uses the same GPUI version and fixture;
it measures native draw wall time, not unique video FPS, GPU completion or a
universal performance guarantee. Release builds and other trees need their own
measurements. The benchmark does not require ScreenCaptureKit video capture.
