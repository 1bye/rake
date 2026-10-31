# Application shapes

Navigation is optional. Standalone and routed apps share services, commands,
managed tasks, application quit guards, and the update controller.

## A standalone window

Use `root` when the view only needs services. Use `root_with` when it also needs
application operations. Both start the same managed runtime:

```rust
use rake::prelude::*;
use rake::gpui::{self, AppContext};

let app = App::new("example.utility")
    .root_with(|runtime, _, cx| {
        let _services = runtime.services();
        let _tasks = runtime.tasks();
        cx.new(|_| gpui::Empty).into()
    });
```

A `RuntimeContext` contains weak runtime handles. Keeping it in a view does not
keep a closed runtime alive. `services()` returns shared application handles;
`tasks()` starts application-owned work; `client()` dispatches registered commands;
`quit()` runs the same shutdown policy used by normal window close and update
restart. A routed page obtains these capabilities from `page.runtime()` and keeps
entry-specific navigation, activities, and guards on `PageContext`.

Closing a standalone window requests guarded application quit by default on every
native platform. Choose `.close_policy(rake::ClosePolicy::KeepRunning)` explicitly
for a tray/background application. An OS force quit, crash, or process kill cannot
be guarded. Route-only settings (`initial`, `entry_guard`, `web`, and
`workspace_chrome`) require `routes`; standalone startup rejects them instead of
silently ignoring them. `runtime_config` configures operation/work bounds for both
shapes; `router` remains a compatible spelling.

## Background and headless work

`App::background(|runtime, cx| { ... })` starts the native GPUI event loop and
managed runtime without opening an initial window. The GPUI app context can own a
tray or other platform integration. Browser hosts reject this mode. Retain any
platform handles in an application-owned global so they live until shutdown.

For a worker with no GPUI event loop, construct `rake::runtime::Runtime` with an
explicit `Services` composition and `RouterConfig`, register commands/quit guards,
and drive its `run()` future on the host's executor. It reuses the application
scheduler. GUI capabilities remain unavailable. A headless runtime does not invent
an executor or silently create a window.

## Shutdown checks

`.quit_guard(|context| async { ... })` registers an application-wide asynchronous
check. Return `Ok(())` after saving or explicit approval, or `Err(GuardReason(...))`
to keep the app running. Checks run after existing finite work has settled and
before application resources stop. Cancellation drops the pending check; it cannot
undo an external save already performed. A guard that waits for user input must
recheck its own mutable model before approving. Per-document revision checks and
discard choices continue to belong to `page.entry()`.

See [routing](routing.md), [managed work](managed-work.md),
[guarded quit](guarded-quit.md), and [distribution](distribution.md).
