# zgx spike: a framework-free Rake core

The standalone zgx spike lives on the separate `t3/split-rake-core-gpui`
branch. It used zgx 0.1.3 for its macOS window and depended directly on
[rake-core](../../crates/rake-core).
The core builds on macOS and for `wasm32-unknown-unknown`. GPUI and zgx
share route mount and guarded close contracts. GPUI owns its window
registrations, native lifecycle callbacks and frame acknowledgements for reload.
The remaining view boundary is how authored views acquire, update and release
backend state.

## What builds without GPUI

- Protocol and clients, HTTP and diagnostics, resources, build/distribution,
  updates, autostart, tray and shortcuts already have independent crates.
  The tray and shortcut integration in the Rake facade still requires GPUI.
- The core owns typed routes and the mountable `Router`, `Runtime`, guarded
  quit, command schemas and dispatch, local IPC, managed tasks, services,
  storage, preferences, diagnostics and checkpoint data. Portable loader state
  is available from the core and the GPUI view binding is in the integration.
- The spike composes `Services` with HTTP, a diagnostic collector and a
  typed store. It opens `Destination::Status`, starts a managed task on
  click, observes its terminal state, and exposes the stored result through a
  real headless command. `Router::with_route_mount` creates its own scheduler
  collector, so the UI reads HTTP diagnostics from the service collector.

The [`rake-app` facade](../../crates/rake/src/lib.rs) forwards the established
features and paths. Default builds expose the [GPUI integration](../../crates/rake-gpui),
including the application factory and existing view macros. Headless consumers
can depend on the core directly or turn off default features on the facade.
Proc macros still resolve the facade first, including renamed dependencies.

## Adapter gaps

| Capability | Current boundary |
| --- | --- |
| Application lifecycle, real windows and window state | `App` starts GPUI and owns its factory callbacks. Standalone windows use `cx.open_window`, and the window-state observer stores GPUI entities and handles. **Window registration and guarded close addressed.** GPUI owns the async contexts and observers, and the router calls a framework-free host binding for removal and shutdown. The zgx spike forwards guarded close and shutdown. A reusable zgx integration still needs window visibility, activation policy and committed application quit. [App](../../crates/rake-gpui/src/application.rs#L101), [run](../../crates/rake-gpui/src/application.rs#L848), [window](../../crates/rake-gpui/src/application/managed/window.rs#L124), [observer](../../crates/rake-gpui/src/window_state/native.rs#L30), [WindowHost](../../crates/rake-core/src/router/types.rs) |
| Routed presentation | **Addressed.** `RouteMount` and `MountedRoute` work without GPUI. The GPUI adapter delegates to `RouteViews` and the outlet projects the retained `AnyView`. zgx mounts hidden trees once, toggles display on presentation changes and explicitly unmounts on disposal. Headless tests verify event order and exactly-once disposal for tab close, history truncation, window close and provisional rollback. [Mount contract](../../crates/rake-core/src/router/mount.rs), [GPUI adapter](../../crates/rake-gpui/src/router/view.rs) |
| Tasks and async view state | `TaskScope` can run on a local executor today. `RuntimeContext::action` and `load` require `gpui::Context<V>`; the implementation observes an entity and calls `cx.notify()`. Host visibility and shutdown observation use GPUI windows, subscriptions and tasks. [Context](../../crates/rake-gpui/src/runtime.rs#L183), [binding](../../crates/rake-gpui/src/loading/binding.rs#L147), [host observation](../../crates/rake-gpui/src/tasks/host.rs#L4) |
| Inspection and devtools | Diagnostics, IPC and wire snapshots are reusable. Element capture wraps `gpui::Element` and intercepts layout, prepaint and paint. Focus, scrolling and style edits hold GPUI handles and style refinements. [Capture](../../crates/rake-gpui/src/inspection/view.rs#L407), [wrapper fields](../../crates/rake-gpui/src/inspection/view.rs#L82) |
| Development reload | **Mount, restore and UI quiescence boundary addressed.** The router retains the capture and restore transactions and reports readiness after portable managed tasks drain and the host reports its UI ready. The GPUI integration tracks frame acknowledgements and drives startup restore and checkpoint cleanup. A zgx host needs its own restart wiring and can supply a first-frame signal if required. [Restore](../../crates/rake-core/src/router/runtime/reload.rs), [startup](../../crates/rake-gpui/src/application.rs), [notice](../../crates/rake-gpui/src/reload/host.rs) |
| Menus and shortcuts | Commands and their presentation data are reusable. Rake's menu resolver produces `gpui::MenuItem` and `gpui::Action`, then registers GPUI key bindings. GPUI supplies validation and resolves the keystrokes for registration. [Resolver](../../crates/rake-gpui/src/menus.rs#L230), [bindings](../../crates/rake-gpui/src/menus.rs#L370), [validation](../../crates/rake-core/src/control/commands.rs#L294) |
| Theme, widgets and `view!` | Theme installation uses `gpui::App` and tokens use GPUI types. Codegen emits GPUI builders, focus and event bindings, style traits and inspection wrappers. [Theme](../../crates/rake-ui/src/theme.rs#L13), [builders](../../crates/rake-view-syntax/src/view.rs#L987), [style presets](../../crates/rake-view-syntax/src/view/preset.rs#L57), [instrumentation](../../crates/rake-view-syntax/src/view/inspection.rs#L116) |

## Option 1

- Keep route identity, admission, command/task policy, typed services and
  checkpoints in a core crate. Reuse `WindowHost` for guarded close and
  optional additional windows. Expose the framework-free lifecycle and reload
  hooks needed across crate boundaries. A host reports visibility and polls
  the local driver on its own loop.
- Both backends now use the route mount contract. Preparation
  receives the typed destination, service registry and cancellation token.
  Mount consumes prepared data with the window and entry IDs and weak
  entry and navigation handles. The router retains the returned mount across
  history, calls its presentation hook after commit and disposes it on
  rollback or removal.
- Keep observation clients on owned protocol snapshots. An optional inspection
  provider returns stable node keys, revisions and geometry, and advertises
  supported operations such as focus, picking and style edits. A menu adapter
  can project command state and dispatch stable command IDs through the registry.
- The GPUI builder, routing views, theme, native bindings and existing
  macros are exposed by `rake-gpui`, preserving the current default facade paths
  through reexports. `rake-zgx` would map the same lifecycle to zgx windows
  and retained view scopes, and map task and route changes to signals.
- Share the `view!` parser and syntax validation and give each backend its
  own lowering. Keep GPUI's frame-time builders and event contexts. zgx
  lowering needs mount-time construction, signal-aware text/styles,
  retained handler and subscription lifetimes, and keyed reconciliation
  for changing branches and lists. Specify which expressions capture once
  and which are re-evaluated. Preserve stable data keys, authored source
  spans, and explicit per-backend extensions for widgets and handlers.

The zgx spike uses `Ui::mount` to add retained roots without replacing the
document. In zgx 0.1.3, `WindowContext::render` sets up desktop services such as
clipboard, dialogs and scoped tasks and also replaces root content. A reusable
zgx route outlet needs to preserve those services when it attaches a new subtree.

## Remaining split work

- Extract an opt-in zgx integration from the standalone spike. Keep zgx
  dependencies out of the default GPUI path and verify independent and combined
  dependency resolution. Validate the browser adapter separately.
- Share view syntax validation and build the two backend-specific lowerings
  and view bindings described above.

## Risks and effort

| Effort | Remaining work |
| --- | --- |
| Medium | Build a reusable zgx host around the spike with visibility, application quit, reload startup and task observation. |
| Large | Define backend-specific view state and lower `view!` for retained, signal-driven trees. Dynamic branches and lists need keyed reconciliation and effect lifetimes. Port applicable widgets, accessibility and inspection support. |

- Cargo features unify additively. Separate integration crates keep each
  backend opt-in for a downstream application. The spike's own lockfile
  isolates wgpu and the zgx taffy fork from the main lockfile. Its resolution
  needed a wider `blocking` range for zgx-desktop's Linux-only 1.6.2
  pin even on macOS. This workspace requires 1.7.
  Test dependency resolution again before combining both backends in one app.
- The core WASM check establishes buildability. `zgx-dom` needs its own
  spike for DOM mounting, browser history, storage, user activation and
  inspection. Check the browser adapter's version and capabilities separately.
  [zgx browser overview](https://github.com/zeronsh/zgx#what-it-has).
