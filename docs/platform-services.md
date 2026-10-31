# Platform services

`runtime.services().platform()` and `page.services().platform()` return the same
application-owned host services. Command handlers can use
`context.services.platform()?`. Bare registries report a missing standard service.

Pickers, clipboard requests and browser downloads start synchronously. They are
intended for click/key handlers, not automatic route loaders, because browser user
activation may expire before an asynchronous task starts. Await the returned request
in view/application-owned work. Dropping observation cannot undo an OS action.

| Service | Native | Browser |
| --- | --- | --- |
| `pick_file` / `FileHandle::read(limit)` | System picker, bounded off-thread read | File input, bounded Blob read; cancel returns `None` |
| `export_file` | Save picker and synced atomic replacement | Download requested; no false claim that a file was saved |
| Clipboard text | GPUI system clipboard | Async Clipboard API; denial is visible; accept DOM paste separately |
| `preferences(namespace)` | Atomic single-file UTF-8 replacements off-thread | Atomic localStorage replacements per key, subject to quota/eviction |
| `open_url` | Delegates to the OS | Opens HTTP(S)/mailto from a user event; popup denial is visible |
| URL activation inbox | GPUI host callbacks and explicit `receive_open_url` | Explicit activation; ordinary app URLs stay with the router |

`capabilities()` is a runtime snapshot. Availability, permission and the need for a
user gesture are independent. An available operation can still fail. Browser
`clipboard_permission(write)` returns `Unknown` when a permission query is absent;
clipboard access may still work. `browser::text_from_paste_event` accepts plain text
without a separate clipboard prompt. Do not suppress the paste shortcut before the
browser can deliver its paste event.

Preferences are small settings, not transactional documents or a notebook database.
Namespace/key names accept 1–128 ASCII letters, digits, dots, underscores and hyphens,
excluding `.` and `..`. Use your reverse-domain app ID as the namespace. Native roots
are Application Support, APPDATA or XDG_CONFIG_HOME (falling back to ~/.config).
`Preferences::at` selects an explicit native root. Native writes sync the temporary
file before replacement; directory power-loss durability is not promised. Read/modify/
write and multi-key changes are not transactions. Notes retains its stronger
document-specific locking, revision and recovery implementation.

URL activation has one bounded, shared inbox (32 pending items); keep one managed
consumer. Receiving an activation does not validate application-specific arguments
or navigate automatically. Custom launchers can call `receive_open_url` with their
startup arguments. Registration metadata alone cannot supply a running event loop.

The adapter does not launch browser helper processes or invent native paths for
browser files. Native GPUI clipboard/URL methods have no system success receipt;
successful delegation is the documented boundary.
