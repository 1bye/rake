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

Inside a user event, `runtime.action(cx, initiated_future)` binds one finite action
with automatic redraw/cancellation; `page.action` provides the entry-owned version.
Start gesture-sensitive browser operations before handing their future to Rake.
Actions do not automatically retry. See [platform services](platform-services.md).

Inside an entity constructor, `runtime.load(cx, factory)` binds an arbitrary
async value to that view; `runtime.bind_updates(cx)` observes the shared updates
controller. Both arrange redraws and cancellation internally. Routed pages can
use `page.load` for additional retained data and `page.bind_updates` for
visibility-scoped update observation. For initial route preparation, register a
`LoadedPage` with `loaded(PageType)` instead. See the [foundation example](../examples/foundation/src/lib.rs)
and [desktop example](../examples/desktop/src/main.rs).

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

For example, background setup can retain a GPUI-owned task and request the same
guarded quit used by a windowed app:

```rust,no_run
struct TrayTask { _task: rake::gpui::Task<()> }
impl rake::gpui::Global for TrayTask {}

let app = rake::App::new("example.tray")
    .background(|runtime, cx| {
        let task = cx.spawn(async move |_| {
            // Perform tray/application work, then:
            // drop(runtime.quit(rake::quit::QuitOptions::default()));
        });
        cx.set_global(TrayTask { _task: task });
    });
```

`TrayTask` is an application-owned `gpui::Global` that holds the task. A
standalone window can instead choose its close behavior explicitly:

```rust,no_run
let app = rake::App::new("example.tray-window")
    .root_with(|runtime, _, cx| {
        let _services = runtime.services();
        cx.new(|_| rake::gpui::Empty).into()
    })
    .close_policy(rake::ClosePolicy::KeepRunning);
```

`KeepRunning` is for an intentional tray/background lifetime; the default
`ClosePolicy::Quit` runs quit guards and stops managed work when the window
closes. `close_policy` applies to standalone windows, not routed workspaces.

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

## Fallible startup

`#[rake::app]` accepts either `App<R>` or `Result<App<R>, E>` where `E` implements
`Display`. A factory can use `?` for opening settings, registering services, or
other initialization. Existing infallible factories continue to work.

Use `try_root` or `try_root_with` when constructing the initial view can fail;
the callback returns `Result<AnyView, MountError>`. Native root/workspace and
initial-window failures return `AppError::Mount` or `AppError::Window` after the
host shuts down, so the generated entry point exits unsuccessfully. Invalid
explicit initial destinations fail validation before the UI starts. Initial page
mount failures remain navigation failures and use the routing recovery UI.

Browser graphics startup is asynchronous. When a document is available, failed
factory/root/window startup displays an alert with a reload button, including when
no GPUI surface could be created. Later rendering/GPU failures remain platform
runtime errors rather than synchronous `App::run` errors.

Run `python3 scripts/test-app-startup.py` on a native desktop session to verify the
real event-loop failure boundary. It intentionally launches a failing fixture and
checks the error and exit status; it is separate from the portable unit suite.

## Services and persistent layouts

Repeated `app.services()` calls and application startup share the configured HTTP
transport. Changing `.http(...)` or `.diagnostics(...)` creates a new transport;
existing service snapshots remain valid. Registries returned by `Services` also
carry the standard HTTP and diagnostic handles for command handlers. A bare
`ServiceRegistry::new()` remains a custom-only composition and reports a missing
standard service explicitly.

Routed apps can install `.layout(...)` or `.try_layout(...)` after registering
routes. The layout is one retained entity per window; pages own only their route
bodies and entry-scoped work. Use `layout.outlet()` in a sidebar/dashboard shell,
and `layout.runtime()` for application operations. `layout.router()` is a weak
upgrade and returns `Result<Router<_>, OperationError>`; an owner that is already
stopping can therefore fail to upgrade, while `layout.outlet()` simply renders
no retained page after the runtime has gone away. Same-type `replace_routes`
keeps the layout and guards. Registering an entry guard or layout before the
route type transition returns an explicit builder-order error at startup.

`python3 scripts/test-app-layout.py` runs the native layout fixture. It verifies
persistent shell state, shared service/transport identity, history navigation, and
rollback when a secondary window's layout fails. It requires a desktop session.

When supplying an update controller manually, its immutable binding must agree
with any registered update service and compiled policy. Packaged descriptors are
validated even when both services were supplied manually. Controller-only
registration supplies its own service; conflicting combinations fail at startup.
