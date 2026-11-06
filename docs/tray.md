# Tray icons

Enable `rake = { ..., features = ["tray"] }`. Rake owns a collection of independent
native items for the application, including after the last window closes. macOS
uses AppKit status items, Windows uses Shell notification-area icons, and Linux
uses StatusNotifierItem/DBusMenu. No GTK, Tokio, second UI loop, devtools connection,
or user-owned keepalive global is required.

Run the [workbench](../examples/tray/src/main.rs):

```sh
cargo run -p rake-tray-example --locked
```

The backend implementations are broader than Rake's platform qualification:
Windows/Linux desktops remain unqualified. The detailed
[native inventory and dispositions](tray-native-api.md) records unavailable
features as well as implemented ones. The [evidence report](../.agents/reports/tray-implementation.md)
separates real native, real D-Bus, TestPlatform and compile coverage.

## Declare independent items

```rust,no_run
use rake::tray::{Image, ImageRepresentation, Menu, MenuItem, Tray};
# fn main() -> Result<(), Box<dyn std::error::Error>> {
let image = Image::new(ImageRepresentation::rgba(18, 18, vec![255; 18 * 18 * 4])?);
let app = rake::App::new("example.tray")
    .tray(Tray::new("main").icon(image.clone()).tooltip("Main")
        .menu(Menu::new().item(MenuItem::new("open", "Open"))
            .item(MenuItem::quit("quit", "Quit"))))
    .tray(Tray::new("status").icon(image).tooltip("Independent status"))
    .background(|_, _| {});
app.run()?;
# Ok(()) }
```

IDs are application-scoped, case-sensitive ASCII names (1–128 bytes, letters,
digits, `.`, `-`, `_`, `:`). Duplicate live IDs fail. Each recreation has a new
registration generation; old handles and queued selections cannot target it.
AppKit autosave keys and default Windows GUIDs include both application and item
identity. Reusing an ID is deliberate persistence, not a second live registration.
The OS controls placement, ordering, overflow, pinning and whether an item is shown.

A native item needs an image; AppKit also allows text-only items through
`macos::Appearance`. Images contain validated straight-alpha sRGB RGBA8 rasters,
with up to 16 sizes. Supply 18/36-point-scale assets on macOS and suitable 16/20/
24/32/40/48 pixel variants for Windows. AppKit retains multiple representations;
Windows selects for effective DPI and refreshes on display/settings changes.
Native names accept portable fallbacks. Template images are AppKit-only.
Supply a meaningful tooltip/accessibility label. Empty native names fall back to
the stable item ID (Windows clips that fallback to its native tooltip buffer).

## Dynamic access and receipts

`runtime.trays()` returns a weak collection. In a foreground event callback,
`rake::tray::collection(cx)` provides the same collection. Use `create`, `get`,
`items`, `set_icon`, `set_tooltip`, `set_menu`, `update`, `show`, `hide`, and `remove`.
Dropping a handle does not remove an item. `remove()` invalidates that generation
immediately; its receipt waits for native cleanup.

Operations are admitted synchronously, with at most 64 in flight. Await their
`Request<T>` receipts to inspect errors and `Applied { revision, superseded }`.
Dropping a receipt cancels observation, not admitted work. Rake's foreground driver
owns work and reports failures through `OperationFailed` events. Native calls can
fail after validation; requested and applied revisions are separate evidence.
A receipt proves native adapter application/export, not visible pixels or host
acknowledgement of presentation. Failed multi-property operations do not promise
rollback; read state and submit a corrected specification.

Custom users of the GPUI-independent `rake-tray` crate must drive `Host::drive()`
on the native UI thread. `Host::flush()` finishes admitted work for finite hosts.
Explicit `shutdown().await` stops admission and joins cleanup; `Drop` is best effort.
Handles/contexts remain safe after shutdown and return `Stopped`/`Removed`.

`HostPolicy::Wait` permits exporting an SNI item before a watcher exists;
`Require` makes startup fail if it cannot register. With `Wait`, native failures
keep a recoverable registration offline when supported, or skip the item while
letting the main window start. `on_tray_event` receives `OperationFailed` and/or
`HostChanged(false)` plus a diagnostic. Failures before a registration exists use
generation zero. Duplicate IDs and invalid configuration still fail startup.
`Registration` distinguishes creating,
registered and waiting. Requested visibility is distinct from actual native
visibility and user shell policy. SNI `Passive` is a hint, not guaranteed hiding.

## Menus, input and commands

Menu trees support actions, submenus, separators, disabled/hidden ancestors,
checks and radio groups. Only one radio per named group can be selected.
Application state is authoritative: update the menu after a selection; Rake does
not implicitly mutate domain state. IDs are unique across the entire tree. Menu
limits are 1,024 nodes and 32 levels. Labels are plain text; `access_key` explicitly
selects a mnemonic on Windows/DBusMenu. Literal `&`/`_` characters are escaped.
AppKit mixed state works natively; other adapters reject it.

`MenuItem::quit` uses Rake's guarded quit. A declined quit preserves items and
managed work. Native AppKit termination-on-removal is intentionally not enabled.
`App::tray_command(tray_id, item_id, typed_command)` dispatches through the normal
command scheduler and capability checks, without transport retry-key retention.
`RuntimeClient::execute(command)` is the corresponding typed local UI entry point;
`dispatch(command, key)` remains the retry-deduplicated transport entry point.
A `RequiresWindow` command never creates a window implicitly.

`App::on_tray_event` runs on the foreground executor after native callbacks return,
without registry borrows. It can remove/update items, open a window or dispatch a
GPUI action. Primary interaction has one owner: `NativeMenu`, `Events`, or `Popup`.
Changing it (or a Windows GUID/SNI bus-name policy) requires recreation. Context
menus remain distinct from primary activation. Keyboard activations do not invent
pointer buttons. Pointer detail never dispatches semantic Activate a second time.
Input detail and menu open/close/highlight events vary by native host.

AppKit defers updates while its menu tracks. Windows retains a menu snapshot until
tracking returns. Every menu projection has a revision; an old projection cannot
dispatch after a newer request. A bounded 256-event ingress emits `Overflow` if
saturated; the facade closes transient popups and logs the loss. Applications
needing recovery diagnostics should observe that event. This is not a durable
command queue.

`accelerator` parses `Ctrl+Alt+Shift+Super+k` (also `Cmd`/`Option` aliases), with a
single printable key. AppKit installs native menu equivalents. SNI exports the
shortcut for the host to interpret. Windows rejects accelerators: the pinned GPUI
message pump provides no supported TranslateAccelerator hook. A displayed label
would not establish execution. These APIs never register global shortcuts.

## Reopen the main window

`App::main_window(options_factory, mount)` registers a repeatable standalone view
factory sharing one retained runtime/services. It opens immediately unless paired
with `.background(...)`. Choose `.close_policy(ClosePolicy::KeepRunning)` for an
Open/Close/Open tray lifetime. `rake::show_main_window(cx)` or
`runtime.show_main_window(cx)` focuses a live window or recreates a closed one.
Mount failures close the failed window and remain retryable. `root` remains
one-shot, and cannot be combined with `main_window`; routed workspaces use their
existing reveal/select/window-host path and retain routed close semantics.

macOS menu-only bundle policy (`LSUIElement`, activation policy, Dock visibility)
is an application/bundle choice, separate from tray registration. Trays never
silently rewrite bundle metadata or hide the Dock. Reopening can activate the app;
background icon updates do not steal focus.

## Rich popups

Attach `Popup::new(width, height, mount)` with `App::tray_popup(id, popup)` and set
that item's primary interaction to `Popup`. Activation toggles it;
`toggle_popup`/`dismiss_popup` also work explicitly. Only one tray popup is open.
Escape, blur, hide, removal, host loss and accepted shutdown dismiss presentation.
A popup does not create a Rake runtime and does not request application quit.
Mount failures remain retryable. Default retention recreates the view each time;
`Retain` is for ordinary GPUI entities that own no resources tied to an old window.
Keep domain state in services when using native child views.

Each show re-queries native icon geometry, display identity, DPI and usable work
area. macOS uses display-local logical coordinates; Windows converts using the
actual monitor's effective DPI. Placement chooses an adjacent panel edge and
clamps to its work area. AppKit highlight follows popup lifetime.

SNI provides no exact tray rectangle; a Wayland host's coordinates/activation token
are not authority to place an unrelated global popup. `RequireAnchor` returns an
error. The default `CenteredWindow` fallback opens an ordinary, activatable window
with compositor-owned placement. The returned `PopupPlacement` states what happened.
Nonactivating windows, native macOS 27 expanded sessions, and surface-anchored
Wayland popups are unavailable with the established host hooks.

## Platform extensions and effects

- `macos::Appearance`: item width, inline/attributed and alternate text, alternate
  images, accessible name/help, visual/interaction disable, user-removal policy.
  `macos_metrics` reports current length/thickness/visibility. `Removal::RequestQuit`
  submits guarded quit and restores the item if quit is declined/cancelled.
  Restore ordinary removal with `show`.
- `macos::MenuAppearance`/`MenuLayout`: attributed titles, subtitles, badges,
  alternates, indentation, state images, tooltips, palette/selection layout and
  explicit context-menu plug-ins. New selectors are checked before use.
- `windows::Presentation`: explicit GUID and standard versus rich-tooltip events.
  `windows::MenuAppearance`: default selection, columns and cascade ordering.
  `legacy_balloon` is optional legacy Shell behavior; it is not modern toast or
  cold-activation support. Quiet time/sound/realtime/icon options are explicit.
- `linux::Presentation`: title/category/status, icon theme path, overlay, attention,
  movie name, rich tooltip and sandbox bus-name policy. Tooltip body is plain text
  escaped for markup interpretation. Activation tokens remain opaque event data.
- `Image::badge` and `progress` explicitly compose portable raster effects.
  `Animation` limits frames, cadence and memory. `animate` owns one playback task
  per ID, respects GPUI reduced motion, and stops on hide/remove/host loss/shutdown
  or external updates. Static items have no periodic timer.

Linux requires a session D-Bus and a StatusNotifierWatcher/panel extension. A bare
GNOME session may need a compatible AppIndicator extension. X11 and Wayland panels
are host-dependent. Watcher replacement on the same bus re-registers items. Whole
session-bus replacement requires removing/recreating items after recovery.
No automatic XEmbed fallback, Ayatana inline-label/order API or GTK loop is installed.
The backend uses unmodified crates.io ksni 0.3.7. Its public `update` API
acknowledges local model processing and hides property/menu signal export errors;
a successful SNI receipt is not proof that D-Bus signals were sent. Rake reports
registration errors, watcher availability callbacks and a closed service handle.
A whole session-bus disconnect may remain undetected by upstream; recreate items
after bus recovery. No complete connection-failure reporting is promised.
See the [unopened upstream proposal](../.agents/reports/ksni-upstream-proposal.md).

## Verification

```sh
cargo test -p rake-tray --locked
cargo test -p rake-app --features tray,view-test-support --lib --locked
cargo clippy -p rake-app --features tray --all-targets --locked -- -D warnings
python3 scripts/test-tray-native.py appkit
python3 scripts/test-tray-native.py sni
python3 scripts/test-tray-native.py gpui
```

The AppKit probe creates real status items on a logged-in desktop. The GPUI probe
opens/closes/reopens native windows, cycles popups and checks declined/accepted quit.
The SNI probe
starts an isolated D-Bus daemon and fixture watcher/consumer; it verifies transport,
not panel appearance. Explicit probes fail when prerequisites are absent. No probe
restarts a user's desktop, Explorer, panel or session bus.

Manual native qualification: run the workbench with two persistent items and a
transient one; exercise keyboard/VoiceOver/Narrator, light/dark/high contrast,
Retina/fractional DPI, localized strings, menu edits while open, popup switching/
Escape/blur, close/reopen, declined/accepted Quit, sleep/wake and display changes.
Measure AppKit/GDI/USER/file-descriptor resources across repeated cycles. Test
Explorer/panel/bus replacement only in a disposable desktop session. Compile checks
and TestPlatform tests do not replace these gates.
