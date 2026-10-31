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
