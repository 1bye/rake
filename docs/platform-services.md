# Platform services

`runtime.services().platform()` and `page.services().platform()` return the same
application-owned host services. Command handlers can use
`context.services.platform()?`. Bare registries report a missing standard service.

Pickers, clipboard requests and browser downloads start synchronously. They are
intended for click/key handlers, not automatic route loaders, because browser user
activation may expire before an asynchronous task starts. Await the returned request
in view/application-owned work. Dropping observation cannot undo an OS action.

`runtime.action(cx, future)` and `page.action(cx, future)` return an `AsyncAction`
with `state(cx)`, `is_running(cx)` and `cancel(cx)`. Rake handles redraw, owner
cancellation and finite-work quit tracking. An action has no automatic retry: a
new browser picker/clipboard attempt needs a fresh user event. For example:

```rust
// Inside a click handler; `copy` is an Option<AsyncAction<(), platform::Error>>.
let request = this.runtime.services().platform().write_clipboard("Hello", cx);
this.copy = Some(this.runtime.action(cx, request));
```

Use `load` for a reusable factory and retry. Do not await guarded quit inside an
action: quit waits for that action to finish. Submit the quit/reload operation,
then finish the action and let the runtime own the handoff.

| Service | Native | Browser |
| --- | --- | --- |
| `pick_file` / `FileHandle::read(limit)` | System picker, bounded off-thread read | File input, bounded Blob read; cancel returns `None` |
| `export_file` | Save picker and synced atomic replacement | Download requested; no false claim that a file was saved |
| Clipboard text | GPUI system clipboard | Async Clipboard API; denial is visible; accept DOM paste separately |
| `preferences(namespace)` | Atomic single-file UTF-8 replacements off-thread | Atomic localStorage replacements per key, subject to quota/eviction |
| `open_url` | Delegates to the OS | Opens HTTP(S)/mailto from a user event; popup denial is visible |
| OS notifications | Native delivery with explicit macOS authorization | Notification API with permission requested from a user event |
| URL activation inbox | GPUI host callbacks and explicit `receive_open_url` | Explicit activation; ordinary app URLs stay with the router |

`capabilities()` is a runtime snapshot. Availability, permission and the need for a user gesture are independent, and an available operation can still fail.

Browser `clipboard_permission(write)` returns `Unknown` when a permission query is absent, and clipboard access may still work. `browser::text_from_paste_event` accepts plain text with no clipboard prompt, so don't suppress the paste shortcut before the browser can deliver its event.

Preferences are small settings, not transactional documents or a database.

- Namespace and key names accept 1–128 ASCII letters, digits, dots, underscores and hyphens, excluding `.` and `..`. Use your reverse-domain app ID as the namespace.
- Native roots are Application Support, APPDATA or XDG_CONFIG_HOME (falling back to `~/.config`). `Preferences::at` selects an explicit native root.
- Native writes sync the temporary file before replacement, but directory power-loss durability isn't promised.
- Read/modify/write and multi-key changes aren't transactions.

Notes keeps its stronger document-specific locking, revision and recovery implementation.

URL activation has one bounded, shared inbox (32 pending items); keep one managed
consumer. Receiving an activation does not validate application-specific arguments
or navigate automatically. Custom launchers can call `receive_open_url` with their
startup arguments. Registration metadata alone cannot supply a running event loop.

The adapter does not launch browser helper processes or invent native paths for
browser files. Native GPUI clipboard/URL methods have no system success receipt;
successful delegation is the documented boundary.


## Native tray icons

Enable `tray` for application-owned multiple icons, native menus and rich popup
windows. This GUI capability is separate from portable notifications and launch
services. Browser/headless GUI calls are explicit errors. See [tray icons](tray.md).

See [OS notifications](notifications.md) for permission results, URL payloads, packaging requirements and the portable Foundation example.

## File drops

File drops reuse `FileHandle::read(max_bytes)` and its size and regular-file
validation. `FileHandle::from_drop(&gpui::ExternalPaths)` accepts exactly one
native path without reading on the UI thread. Empty and multi-file drops fail.
The files example accepts one UTF-8 file up to 128 KiB and refuses replacement
while busy or while a transformation is unsaved.

On Web, `platform::browser::on_canvas_file_drop` registers a canvas-wide target
and returns an owned subscription. Its callback receives a real browser File
handle through `FileHandle`, with the same bounded read as `pick_file`; it never
exposes a synthetic path. Empty or multi-file drops report `InvalidInput`.
Dropping the subscription removes the DOM listeners. Drag-over outlines the
canvas; retain an accessible target label and the import button for keyboard use.

## Global shortcuts

Enable `global-shortcuts`, or `tray` which includes it. Call
`platform().global_shortcuts().register("Ctrl+Alt+k", callback, cx)` from the UI
thread and await its request in application-owned work. The callback receives
`&mut gpui::App` on the UI thread and can dispatch an application command or call
`rake::tray::toggle_popup`. The tray example toggles its rich popup this way.
Keep the returned `GlobalShortcut` alive. Drop stops callbacks and unregisters;
`unregister()` returns an awaitable cleanup receipt and can be called repeatedly.
App quit releases registrations even if their handles remain alive. Cancellation
of a pending registration releases any acquired resources.

Accelerators reuse `rake-tray::Accelerator`: one printable key with unique
`Ctrl`, `Alt`/`Option`, `Shift`, `Super`/`Cmd` modifiers. A global shortcut must
include Ctrl, Alt or Super. Invalid keys, native failures, permission denial and
`platform::Error::ShortcutConflict` are observable errors; no alternative key is
silently registered. Repeated presses may be coalesced while the UI is busy.

- macOS uses Carbon event hotkeys without Accessibility or Input Monitoring
  permission. Printable keys use the current keyboard layout, including implicit
  Shift for symbols. Keys absent from the layout and named keys such as Enter are
  rejected. Layout changes require re-registration. System-reserved shortcuts can fail.
- Windows uses `RegisterHotKey` on a message-only window in the UI message pump.
  ASCII keys are mapped through the current keyboard layout. Held-key repeat is
  disabled. OS-reserved combinations and shortcuts owned by other apps can fail.
- Linux X11 uses passive key grabs on a private connection. ASCII keys use the
  current layout; Alt and Super mean Mod1 and Mod4. Caps Lock and Num Lock are
  handled. Layout/modifier-map changes require unregistering and registering again.
- Wayland uses the GlobalShortcuts desktop portal, never an XWayland grab. The
  compositor may prompt, deny access, or let the user change the preferred binding;
  the portal owns conflict resolution and user-approved bindings. Closing the
  private D-Bus connection ends the session on unregister/drop, including pending
  authorization. A missing portal returns Unsupported. Rake never bypasses it.
- Web, headless builds and hosts without the feature return explicit Unsupported.
  `capabilities().global_shortcuts` reports support. On Linux it starts false;
  await `global_shortcuts().available()` to refresh that shared capability snapshot
  without prompting. Availability is no guarantee that a requested key is free.

Native registration, conflict and cleanup probes run in macOS/Windows CI and
under an isolated X11 display in Linux CI. They do not establish real key presses,
Wayland compositor authorization, or cross-application focus behavior.

## Secrets

`platform.secrets()?` uses the application id configured in `App::new`. A
platform without a configured id returns `Error::MissingApplicationId`. With
`gpui`, call `read(name, cx)`, `write(name, token.into(), cx)` or
`delete(name, cx)` and await the returned `Request`. `Secret` takes ownership
of a `String` or `Vec<u8>` without copying its bytes. Missing reads return
`None` and missing deletes succeed. Calls initiate GPUI tasks immediately.
Native keychain work runs on its background executor and may prompt.
Dropping a request cannot undo an initiated write.

Keep the application id stable across restarts and use non-secret names. Ids
and names must each be 1–128 UTF-8 bytes.
The URL key is `rake-secret://v1/<hex UTF-8 app id>/<hex UTF-8 name>`;
the username is always `rake-secret-v1`. Encoding is case-sensitive and injective.
Each name needs its own URL because GPUI reads/deletes by URL alone. Namespacing
prevents accidental app collisions, not access by another process using the same id.

| Host | Backend and errors |
| --- | --- |
| macOS | System Keychain. Typed cancellation/authentication failure maps to `SecretAccessDenied`; OSStatus -25308 maps to `KeychainLocked` (interaction unavailable, including locked/headless sessions). Other failures are `SecretsUnavailable`. |
| Windows | Credential Manager. Values above 2560 bytes return `SecretSizeLimit`. Typed access-denied errors from reads/deletes map to `SecretAccessDenied`; GPUI converts write errors to text, so those become `SecretsUnavailable`. Missing deletes are normalized from ERROR_NOT_FOUND. |
| Linux | GPUI uses oo7 and the session Secret Service. Typed locked/dismissed errors map to `KeychainLocked`/`SecretAccessDenied`; typed D-Bus ServiceUnknown maps to `NoSecretService`. Other connection/unlock failures return `SecretsUnavailable`; absence cannot always be distinguished from an unreachable session bus. |
| Web | Always `SecretsUnavailable`; no localStorage or other fallback. |

Backend error text is discarded rather than risking content in logs. `Secret`
has redacted Debug, no Display or serialization, and zeroizes its owned bytes on
drop using zeroize. GPUI and OS intermediate copies are outside this guarantee.
`expose()` is an explicit escape hatch: callers must never log, serialize, inspect,
or place those bytes in preferences, view state or diagnostic fields. The service
registers no storage, CDP or DevTools inspection provider and emits no tracing events.
This does not encrypt Store data or implement biometric prompts.

The desktop example has Store, Check and Delete demo-token buttons. Store once,
restart the app, then Check to exercise persistence; Delete afterwards. It displays
only presence and sanitized errors. Never use a production token for this demo.
