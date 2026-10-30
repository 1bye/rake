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

**Select element** reads the deepest inspected boundary using Rake's existing
clipping and observed paint order, without activating the control. The browser
outlines its bounds and displays the Rust construction site, semantic identity,
text and macro-authored style source metadata. **Copy selection** exports that
context as JSON. Scrolling works in both Interact and Select element modes;
selection mode changes only click behavior. Opaque/cached descendants resolve
to an inspected ancestor.
Comment pins and agent integration are later work.

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
