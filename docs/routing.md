# Routing core

The routing core implements the destination, tab, retained-entry, operation,
guard and browser-history contracts in the draft. The executable
[example](../examples/routing/src/lib.rs) mounts real GPUI entities through
`RouteViews` and works on native macOS and WebAssembly.

## Destinations and views

```rust
use rake::prelude::*;

#[derive(Clone, Route)]
enum Destination {
    #[route(id = "home", path = "/")]
    Home,
    #[route(id = "document", path = "/documents/{id}")]
    Document(u64),
    #[route(id = "search", path = "/search")]
    Search { query: String, page: Option<u32> },
}
```

How fields map to a route:

- Explicit IDs are stable external keys.
- A single tuple field binds the single path placeholder.
- Named fields bind matching placeholders, and the remaining fields become query parameters.
- Optional query fields use `Option<T>`.
- Parameter types implement `RouteParam`. `Display` plus `FromStr` supplies the usual implementation for scalars and domain IDs.

Route enums don't need Serde or `Send`.

`Route::encode` produces an owned `RouteAddress`, and `decode` validates it. `to_uri` and `from_uri` share the browser codec. `descriptors` exposes the closed registry and parameter schema.

- Unknown, missing, malformed and duplicate parameters fail explicitly.
- Static paths take precedence over dynamic ones.
- Encoding rejects values shadowed by another pattern, empty path values and dot segments, so a successful encoding round-trips to the same route.
- Overlapping patterns of equal specificity, such as `/files/{id}` and `/{section}/settings`, fail at derive time.

Register page types with an exhaustive destination match:

```rust
App::new("example.notes")
    .theme(Colors::dark())
    .routes(rake::routes! {
        Destination::Home => Home,
        Destination::Document(..) => DocumentPage,
    })
    .initial(Destination::Home)
```

Register `.routes(...)` once, then configure `.initial(...)` and `.entry_guard(...)`.
Use `.replace_routes(...)` to change the factory for the same destination type;
it preserves the initial destination and guards.

`Home` and `DocumentPage` implement `Page<Destination>` alongside GPUI `Render`. `Page::mount(destination, page, window, cx)` returns `Result<Entity<Self>, MountError>` and owns that page's service resolution, typed parameters and initialization.

Register parameters with `(..)` or `{ .. }`, then read the full destination in the page constructor. Registration expands to a checked Rust match with no implicit fallback, so a missing destination fails compilation. The result implements `RouteViews`, which remains available for custom dispatchers and hosts.

`App::theme` (available with `experimental-view`) installs each owned theme once on the UI thread, before the first window or route mounts. You can register it before or after `.routes`, and navigation never reinstalls it.

Ordinary shared components only implement `Render` or `RenderOnce` and needn't depend on `PageContext`. See the [small desktop app](../examples/desktop/README.md) for complete page implementations and app setup.

Use `.layout(|layout, window, cx| cx.new(|_| Shell::new(layout)))` after `.routes(...)` for a persistent application shell. Use `.try_layout(...)`, which accepts `Result<Entity<_>, MountError>`, when initialization can fail.

Rake mounts one layout per window, retains it across route changes, and leaves route bodies in `layout.outlet()`. `LayoutContext` supplies window-bound navigation, shared services and `runtime()` capabilities. Guard decisions, error presentation, reload and shutdown stay with the workspace outside your layout.

`.new_window_options(|window_id| WindowOptions { ... })` configures secondary
native windows. The navigation activation controls focus; `window_options`
continues to configure the initial window. Browser hosts have one physical window
and reject `NewWindow` operations.

The runner installs the default tab strip, route outlet and decision presenter. Domain models may be shared, but each retained entry owns its own view and local state. The browser's URL takes precedence over `initial`.

A custom GPUI host can use `Router::in_window`, own its `run` future, observe `changed`, and compose `TabBar` and `RouteOutlet`. `Router::new` is a headless state model without GPUI resources.

## Tabs, histories and commands

`page.navigator()` binds contextual commands to that screen's original tab.
`page.tab()` and `navigator.tab(id)` target an exact tab. App-level contextual
targets resolve at submission, before any asynchronous guard. Closed targets
produce errors instead of redirecting to the newly selected tab.

| Command | Behavior |
| --- | --- |
| `open(route, NewWindow)` | Creates a native host with a fresh independent tab and root entry. |
| `open(route, NewTab)` | Always creates a fresh tab and root entry. |
| `open(route, CurrentTab)` / `tab.navigate(route)` | Pushes a fresh instance, guarding disposal of forward history. |
| `tab.replace(route)` | Guards and replaces the current instance. |
| `tab.back()` / `tab.forward()` | Moves the cursor and reuses the retained view. |
| `tab.select()` | Changes selection; preserves every history. |
| `reveal(route, scope)` | Selects the most recently selected matching current entry, or creates one tab. |
| `tab.close()` | Guards every retained entry, removes the tab and cancels entry lifetime signals. |

- New tabs preserve all existing tabs, even for identical destinations.
- Hidden views stay retained but don't enter the displayed layout or paint tree.
- `OpenOptions` controls activation and the initial stored mode. Hidden navigation doesn't select its tab.
- A history capacity blocks a push that would exceed it and never silently evicts an entry.

`RouterConfig::default()` leaves history unlimited (`None`). The small desktop example opts into a 64-entry limit with `runtime_config`, so production apps can choose their own policy.

Commands queue eagerly and return cloneable `Operation<T>` handles. `wait().await`
only observes the shared result; dropping a handle does not cancel the command.
Use `Router::cancel(id)` to request cancellation before commit. Conflicting jobs
run in order within their tab; an unrelated tab can progress during a decision.
Reveal lookup and creation are serialized and revalidated before commit.

`Outcome` distinguishes committed, cancelled and blocked operations. Technical failures use `OperationError`.

Snapshots expose operation phases, pending decisions, tab histories, dirty revisions, capacity and host capabilities. Admission and completed-operation retention have configurable count bounds. An evicted completed operation reports `OperationExpired`, but its existing handle still owns its result. IDs belong to one `RuntimeId` and must not be reused across runtimes.

### Route loaders

Use `loaded(PageType)` to opt a route into asynchronous preparation:

```rust
rake::routes! {
    Destination::Home => loaded(Home),
    Destination::Updates => Updates,
}
```

`Home` implements `LoadedPage<Destination>` with owned `Data`, a typed `Error`, `async fn load(destination: Destination, load: LoadContext)`, and `mount(destination, data, page, window, cx)`. Ordinary `Page` implementations work in the same registration.

Loaders can query services, read local databases or files, or prepare domain resources; they needn't use HTTP. They run on the UI event loop with no `Send` requirement, so blocking work must explicitly use `load.background_executor()` or an application worker service.

Admission and guards run before preparation. Rake then awaits the result, revalidates cancellation and entry revisions, and mounts and commits the page. A navigation that creates a native window prepares its data before opening that host.

- Failed or cancelled preparation leaves the existing page, history and URL intact and uses the normal recovery controls.
- Retry reruns admission, guards and loading.
- Prepared data is dropped when the attempt is cancelled or mounting fails.

Each fresh retained entry loads once. Back and Forward reuse its data and view, while entries reconstructed after a process or browser reload prepare again. There's no implicit cross-route cache or background revalidation.

A loader must tolerate repetition on retry. Explicit writes such as document saves and update installation belong in commands or actions. The [desktop Home page](../examples/desktop/src/pages/home.rs) shows worker-backed resource preparation and retained dirty state.

### Admission and navigation recovery

Custom router hosts can register a typed admission callback before starting the
driver. Admission runs before entry allocation, native host creation or page
mounting, and it returns data rather than navigating as a side effect:

```rust,no_run
use rake::prelude::*;

#[derive(Clone, Route)]
enum Destination {
    #[route(id = "home", path = "/")]
    Home,
    #[route(id = "login", path = "/login")]
    Login,
}

let router = Router::<Destination>::new(RouterConfig::default());
router.entry_admission(|context| async move {
    match context.destination {
        Destination::Home => Admission::Redirect(Destination::Login),
        Destination::Login => Admission::Allow,
    }
}).unwrap();
```

The context contains the operation ID, presenting window, resolved target tab, typed candidate destination and an immutable `ServiceRegistry`. Admission may be asynchronous, but must not retain UI borrows.

- A redirect is re-admitted as part of the same operation. `RouterConfig::max_admission_redirects` bounds loops, eight by default.
- Denials become `Outcome::Blocked`. Technical failures remain `OperationError` values.
- Admission handles destination-bearing Open, Navigate, Reveal and browser Restore operations.

Back, Forward and tab selection reuse retained entries and run entry guards, so use those guards when access must be checked on every entry.

Each route-changing operation also retains a `NavigationStatus`, keyed by runtime, operation, window and destination.

- `Router::navigation(window_id)` exposes pending, failed, blocked, cancelled and committed attempts, independently of the bounded completed-operation table.
- `retry_navigation(&key)` replays the exact action through admission and guards.
- `dismiss_navigation(&key)` removes a terminal status.

Retention rules:

- Retry keys have their own bound, `RouterConfig::navigation_records` (64 by default), so a status can outlive the operation result but not the runtime's navigation history.
- Pending attempts are never evicted.
- The effective terminal limit is at least one, and an empty live window keeps its last recoverable failure even at the limit.
- A successful retry or Dismiss clears that attempt's earlier retry chain. Unrelated failures stay independent.

`App::routes` renders `NavigationBoundary` around the workspace. It shows pending feedback with Cancel, and failed or blocked feedback with Retry and, once a page is visible, Dismiss. It doesn't replace route pages or cache failed data.

A custom GPUI host can render `NavigationBoundary::new(router).in_window(window_id)` beside its `RouteOutlet`, then notify the owning shell after `Router::changed` resolves. An app layout can provide its own feedback from the same statuses and opt out with `.navigation_feedback(false)`.

## Guards and decisions

Register access guards with `App::entry_guard` or before starting `Router::run`.
Register disposal guards on `page.entry().on_dispose`. They receive owned
destination, identity, dirty state, revision and guard kind, and return `Allow`,
`Deny(reason)` or `Ask(DecisionRequest)`. Retaining an outgoing screen while
switching tabs or traversing history does not run its disposal guards.

Call `entry.set_dirty(true)` for every edit, including while already dirty. Each call advances the revision and expires prior decisions.

- Without a custom disposal guard, a dirty entry offers Discard and Cancel. Custom guards own their save policy.
- A decision choice can register asynchronous `prepare` work, such as saving and clearing dirty state. If the revision changed afterwards, the router reruns guards.
- All affected entries are revalidated before anything is disposed.
- Mount failures leave the existing workspace intact.

The default UI calls `navigator.respond(decision_id, choice_id)`; custom presenters
use the same method and snapshot descriptors. IDs and allowed choices are checked.
An edit expires an old decision, while selecting or editing an unrelated tab does
not. Cancellation cannot undo a save or other external effects already performed
by application callbacks.

## Native windows

`App::routes` installs the native window adapter automatically. Every window has
its own selected tab and retained histories while sharing one RuntimeId, services,
command registry, operation scheduler and development-control endpoint.

```rust,no_run
use rake::prelude::*;
use rake::router::{OpenTarget, Router, RouterConfig};
#[derive(Clone, Route)]
enum Destination { #[route(id = "home", path = "/")] Home }
let router = Router::<Destination>::new(RouterConfig::default());
// In a native App::routes host, which supplies the physical-window capability:
let operation = router.navigator().open(Destination::Home, OpenTarget::NewWindow);
// The driver runs eagerly; a GUI handler need not wait here.
```

`PageContext::window_id()` and `EntryHandle::window_id()` identify the originating host.

- `navigator.for_window(id)` binds app-level Open and Reveal. Contextual CurrentTab resolves that host's selected tab at submission.
- A page navigator stays bound to its original tab and window, even when focus or selection changes.
- `RevealScope::Window` restricts reuse to the bound host. Application searches all hosts and activates the matching window.
- Selecting an exact tab activates its host. PreserveSelection avoids activating a newly opened host.

`navigator.close_window()` or `router.close_window(id)` guards every retained entry in that host, including hidden tabs and back/forward histories, using `DisposeReason::CloseWindow`.

- Repeated pending requests join one operation.
- Save, Discard and Cancel use the existing decision engine, and revisions are checked again before atomic disposal.
- Access decisions for NewWindow appear in the originating host before the new window is created.
- Pending decisions activate their presenting native host, including while application quit checks another window.
- While a close is pending, new navigation into that window fails with `WindowClosing`. Other windows continue.
- Denial or cancellation leaves all the window's entries intact.
- Commit cancels entry and tab scopes and closes the physical host. Application-owned tasks and other windows survive.

The native close button follows this operation. Closing the last window leaves the
runtime alive; app-level NewWindow can open another host. Explicit Quit still
guards every window and coordinates application jobs before process exit.

`RouterSnapshot::windows` exposes per-host selection, visibility and pending close;
tabs and decisions carry WindowIds. `snapshot_in_window(id)`, `TabBar::in_window(id)`
and `RouteOutlet::in_window(id)` scope presentation. Default widgets show the initial
host. After closing it, app-level commands must name a remaining host, except
NewWindow which can also create a host when none remain.

Custom native shells install `Router::window_host` before driving the runtime.

- `WindowHost::open` must register the reserved host with `Router::attach_window` before returning.
- `close` must retire the physical host.
- Capture a weak navigator and upgrade it with `navigator.router()` inside host callbacks, rather than retaining the router in its own adapter.
- The embedding app owns one shared driver, redraw observations and explicit quit mode, independently of the first view.

Without a window adapter, `Router::in_window` keeps its application-quit close behavior. Additional default runner windows use GPUI's default window options. OS position and size, and moving tabs between windows, are outside this API.

## Browser history

Use `.web(WebConfig::new().routing(WebRouting::Path))`, or `Hash` for fragment URLs.
Path deployments must serve the app shell for deep links. The example development
server provides that rewrite; missing asset requests still return 404.

Browser history records visible workspace visits independently of tab histories.
Visible open, selection, navigation and per-tab traversal push checkpoints;
initialization, replacement and selected-tab close replace the current checkpoint.
Hidden changes leave the URL alone. Closing the last tab clears Rake's current
checkpoint and shows the root URL.

Browser Back and Forward reuse matching live runtime, tab and entry hints, including duplicate destinations. Stale hints fall back to a normal guarded reveal or creation from the checkpoint's complete destination.

- Restoration replaces state without pushing a feedback visit.
- Newer traversals supersede older pending ones.
- A denied, cancelled or failed traversal reconciles the URL to the committed screen.
- History API failures appear in `history_error` without rolling back a committed tab operation.

Checkpoints are versioned destination records, not durable view or draft storage.
Reload reconstructs the URL destination in a fresh runtime. Browser termination
and cross-document navigation cannot promise asynchronous save completion.

## Current limits and evidence

Web has one logical host. `NewWindow` reports `UnsupportedCapability` there, and on headless or custom hosts without a window adapter; it never falls back to NewTab. Snapshots advertise the targets actually supported.

`TabMode` drives Foreground and Live activity (see [managed work](managed-work.md)). [Guarded quit](guarded-quit.md), [runtime control](runtime-control.md), [DevTools](devtools.md) and [development reload](development.md) all use this routing core. Durable workspace persistence and native OS deep-link registration are separate capabilities.

Native multi-window behavior has been run on macOS. Other native platforms are unverified.
