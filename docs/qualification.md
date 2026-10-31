# Release qualification

Run `scripts/check-readiness.py` against the release revision and preserve its JSON
report. Do not combine a passing compile check and a manual checklist into a claim
that every desktop/browser journey passed. The focused cross-platform workflow is
manually dispatched once after a change set; signing/install workflows remain opt-in.

## Automated boundaries

| Check | What it establishes | What it does not establish |
| --- | --- | --- |
| Portable runtime tests | Guards, tasks, history, service identity, failure/cancellation | OS windows, GPU, browser permissions |
| GPUI TestPlatform tests | Real layout/focus/action algorithms and retained view state | Native rendering, IME or an OS accessibility adapter |
| Native startup/layout smoke | Actual event loop, startup failures, persistent shell and host rollback | Every window manager or display configuration |
| Platform file/preferences tests | Bounded reads, atomic replacement, failure preservation, FIFO rejection | OS picker UI |
| JS adapter/worker/cache tests | Denial, quota, cancellation, timeout and cache policies | Actual browser lifecycle |
| `test-web-platform.mjs` | Actual Chrome file selection/Unicode reads, clipboard with automation permission, workers, offline multi-build lifecycle, Rust worker-drop cancellation and guarded reload | GPUI canvas rendering, manual permission prompts or screen-reader usability |
| Windows/Linux app-readiness matrix | Portable tests and native target compilation, platform JS boundary tests | Native installer/signing or real GUI qualification |

The real browser test needs Chrome, matching wasm-bindgen-cli, Rust's Web target,
Python and Node 22+. Set `CHROME_BIN`/`WASM_BINDGEN` for nonstandard tool locations.
It uses a temporary browser profile/loopback origin, restores clipboard text after
the fixture operation, and destroys its worker/browser/server resources. A missing
prerequisite fails explicitly. Its output includes the actual browser version.

## Visibility contracts

`rake::tasks::visibility_support()` describes the selected built-in adapter:

- macOS measures AppKit visibility, minimization and occlusion.
- Windows measures visible/minimized state and DWM cloaking, including hidden virtual
  desktop surfaces. It does not measure overlap by other windows.
- Web measures document visibility and pagehide/pageshow. The browser can suspend
  the event loop despite a pending timer or task.
- Linux currently reports `Unknown`. X11 and Wayland/compositor support need a host
  visibility signal, not an inference from focus. Custom hosts can supply one via
  `Router::set_window_visibility`. Unknown permits activity and remains observable.

## Real-host beta gates

Record OS, browser, display server, GPU, scale factors, locale and revision for each
run. These checks require the actual devices/assistive tools and remain release gates:

1. **Accessibility:** navigate the complete app with keyboard only, then VoiceOver
   on macOS, NVDA on Windows and the supported browser/screen-reader combinations.
   Verify names/roles/state announcements, reading order, focus restoration, dialog
   containment, disabled controls and update/loading/error announcements.
2. **Text input:** Latin, CJK IME composition, emoji/combining characters, selection,
   cut/copy/paste, undo/redo, dead keys and keyboard layout changes in Notes. Check
   browser paste events separately from programmatic clipboard permission.
3. **Displays/windows:** resize, minimize/restore, fullscreen, 100/125/150/200% scale,
   move between differently scaled monitors, background/foreground and close/reopen.
   Test supported X11 and Wayland environments independently.
4. **Recovery/update:** interrupt save, kill/reopen, deny and approve shutdown,
   retry failed downloads, upgrade old persisted formats and restore durable drafts
   after browser reload. Verify old tabs keep their immutable resources during deploy.
5. **Sustained workloads:** measure cold start, frame latency and retained memory
   while editing a large document, navigating bounded histories and repeatedly
   importing/exporting files. Capture before/after numbers with the same workload.
   An empty or synthetic CPU benchmark is not a GPU/frame performance result.

The experimental markup surface is not advertised as screen-reader-qualified until
those runs pass. New convenience APIs do not eliminate these host-specific checks.
