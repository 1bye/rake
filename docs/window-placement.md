# Native window placement

Window placement is opt-in. Without a policy, Rake keeps the existing window
options on every launch. The desktop example enables placement for its main
window:

```rust,no_run
let preferences = rake::platform::Platform::default()
    .preferences("example.app")?;
let placement = rake::window_state::WindowState::new(preferences);
let app = rake::App::new("example.app")
    .window_state(placement.clone());
```

Use a preference namespace belonging to your application. The policy stores a
versioned JSON document under `rake-window-state-v1`, using the existing atomic
preferences service. It does not restore windows, tabs, or routes that your app
does not open. Configure `window_options` as usual for first-launch defaults.
Missing records, invalid JSON, invalid dimensions, and unknown schema versions
use those defaults. An invalid record does not discard other valid windows.
Read/write failures warn without preventing the app from running.

## Window identities and opt-out

The default policy gives only the initial window the key `main`. Secondary
windows opt out until you supply a mapping:

```rust,no_run
use rake::router::WindowId;

# fn configure(preferences: rake::platform::Preferences) {
let placement = rake::window_state::WindowState::new(preferences)
    .keys(|id| match id {
        WindowId(1) => Some("main".into()),
        // Resolve other IDs through your app's window-role/document registry.
        _ => None,
    });
# }
```

Runtime window IDs are not durable identities. Map each ID to an application
role or document identity established before opening that window. Do not derive
saved keys from opening order. Return `None` for temporary windows, dialogs, or
any window that should always use its defaults. Keys are case-sensitive,
nonempty strings of at most 128 UTF-8 bytes. Two simultaneous windows cannot
share a key: the second warns and opts out. Keys are fixed for each opening.
The same policy applies to routed secondary windows and to a main window reopened
from a background application. Custom windows opened directly through GPUI are
outside the `App` policy.

## Displays, scaling, and window modes

Rake saves normal restore bounds in logical pixels, a stable display UUID when
available, and windowed/maximized/fullscreen mode. Native titlebar height is
accounted for separately when the adapter opens content bounds but reports frame
bounds, preventing a window from growing on each launch. The backing scale is not
multiplied into saved coordinates. Moving between 1x and 2x displays therefore
preserves logical size; a smaller logical work area clamps the size as needed.
Normal bounds are retained while maximized or fullscreen so leaving those modes
does not replace the original restore size with the full display size. Where the
native adapter does not expose unmaximized restore bounds, Rake retains the last
windowed bounds settled for 250 ms; a resize immediately followed by maximizing
may therefore restore the preceding settled size. Startup mode requests are
protected from intermediate animation frames. If the host has not entered the
requested mode after two seconds, tracking follows its actual mode.

On reopening, Rake matches display UUIDs rather than transient display indices.
Partially off-screen bounds are clamped fully into that display's visible work
area, excluding system UI where GPUI reports it. If the display has disconnected
or the saved bounds are entirely off-screen, Rake uses the app's default display
and placement. Off-screen defaults are centered and oversized bounds are reduced
to fit. With no available display Rake leaves placement to the host. Window mode
is retained when falling back to a different display. Display changes during a
session update the saved display and bounds through GPUI's observers; restoration
rechecks the currently available displays at each opening.

The native window manager makes the final placement decision. In particular,
Wayland compositors commonly prohibit absolute positioning. Rake can request the
saved size and mode, but the compositor chooses the position and may override
size, focus, or fullscreen. Window minimum sizes and native decoration constraints
can also override the requested bounds. The policy uses GPUI's validated bounds
and mode APIs; opaque native workspace archives are not persisted.

## Saving and resetting

Move, resize, display, and mode changes are coalesced for 250 ms. Continuous drags
save at most once per interval rather than delaying forever. All writes are
serialized and use the latest records so two windows cannot overwrite each
other's updates. Closing a window requests an immediate final write. Application
quit captures every remaining window and awaits the final write within GPUI's
shutdown timeout. Crashes, forced termination, disk failures, or shutdown timeout
can lose the latest change. File I/O runs through the preferences worker pool.

Await `placement.reset("main")` to forget one record. This does not move an open
window; its next change or final close/quit may save its current placement again.
To return to defaults permanently, opt that window out as well. A reset before
startup loads existing records first, preserving other windows. Resets serialize
with outstanding saves so an older queued write cannot undo a completed reset.

## Web

The browser owns its window. `App::window_state` and `WindowState::reset` are
no-ops on Web: they do not read or write placement preferences or attempt to
resize the browser. The same builder chain can be shared with native targets.

## Verification status

Pure tests cover disconnected and off-screen displays, negative desktop
coordinates, smaller work areas, mixed logical display sizes, all window modes,
corrupt records, and schema versions. Headless window tests exercise GPUI's test
platform; they do not establish behavior of a real window manager.

Native verification on macOS 26.6.2 (25G83), Apple silicon, used a built-in
3456×2234 display at logical 1728×1117, 2x, with a second 3840×2160 display at
logical 1920×1080, 2x, connected. The desktop smoke build reopened a 720×480
content area at (200, 150) across three launches with changed fallback defaults;
the actual native frame remained 720×512. Saved maximized and fullscreen records
restored their modes while retaining those normal bounds. Injected off-screen
bounds and a missing display UUID fell back to a visible display. The latter
simulates disconnection in saved data; it is not a physical unplug/replug test.

Build with `cargo build -p rake-desktop-example --features native-smoke`. Set
`RAKE_DESKTOP_SMOKE_PREFERENCES` to an isolated temporary directory. The smoke app
prints `RAKE_DESKTOP_SMOKE_BOUNDS`, presents a frame, and quits cleanly. Set
`RAKE_DESKTOP_SMOKE_ALTERNATE_BOUNDS=1` on the first launch to start at (200, 150)
with a 720×480 content area; omit it on subsequent launches to verify that saved
placement takes precedence over different defaults.

Windows, Linux X11, and Linux Wayland native reopening still require verification
before issue #14's cross-platform acceptance is complete. Record each OS version,
display resolution, scale, and monitor layout with the results. A successful
compile or headless test does not establish native window-manager behavior.
