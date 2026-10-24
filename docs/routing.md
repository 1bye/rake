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

Explicit IDs are stable external keys. A single tuple field binds the single
path placeholder. Named fields bind matching placeholders; the remaining fields
become query parameters. Optional query fields use `Option<T>`. Parameter types
implement `RouteParam`; `Display` plus `FromStr` supplies the usual implementation
for scalar values and domain IDs. Route enums do not need Serde or `Send`.

`Route::encode` produces an owned `RouteAddress`; `decode` validates it.
`to_uri` and `from_uri` share the browser codec. Unknown, missing, malformed and
duplicate parameters fail explicitly. Static paths take precedence over dynamic
ones. Values shadowed by another pattern, empty path values and dot segments are
rejected when encoding, so a successful encoding round-trips to the same route.
`descriptors` exposes the closed registry and parameter schema.

`App::new(...).routes(AppRoutes).initial(Destination::Home)` installs the default
tab strip, route outlet and decision presenter. `RouteViews::mount` receives a
destination and owned `PageContext`, and creates a fresh `AnyView`. Domain models
may be shared, but each retained entry owns its own view and local state. The
browser's URL takes precedence over `initial`. A custom GPUI host can use
`Router::in_window`, own its `run` future, observe `changed`, and compose `TabBar`
and `RouteOutlet`. `Router::new` is a headless state model without GPUI resources.

## Tabs, histories and commands

`page.navigator()` binds contextual commands to that screen's original tab.
`page.tab()` and `navigator.tab(id)` target an exact tab. App-level contextual
targets resolve at submission, before any asynchronous guard. Closed targets
produce errors instead of redirecting to the newly selected tab.

| Command | Behavior |
| --- | --- |
| `open(route, NewTab)` | Always creates a fresh tab and root entry. |
| `open(route, CurrentTab)` / `tab.navigate(route)` | Pushes a fresh instance, guarding disposal of forward history. |
| `tab.replace(route)` | Guards and replaces the current instance. |
| `tab.back()` / `tab.forward()` | Moves the cursor and reuses the retained view. |
| `tab.select()` | Changes selection; preserves every history. |
| `reveal(route, scope)` | Selects the most recently selected matching current entry, or creates one tab. |
| `tab.close()` | Guards every retained entry, removes the tab and cancels entry lifetime signals. |

New tabs preserve all existing tabs, even for identical destinations. Hidden
views remain retained but do not enter the displayed layout or paint tree.
`OpenOptions` controls activation and the initial stored mode. Hidden navigation
does not select its tab. A history capacity blocks a push that would exceed it;
it never silently evicts an entry. Retention is unlimited by default.

Commands queue eagerly and return cloneable `Operation<T>` handles. `wait().await`
only observes the shared result; dropping a handle does not cancel the command.
Use `Router::cancel(id)` to request cancellation before commit. Conflicting jobs
run in order within their tab; an unrelated tab can progress during a decision.
Reveal lookup and creation are serialized and revalidated before commit.

`Outcome` distinguishes committed, cancelled and blocked operations; technical
failures use `OperationError`. Snapshots expose operation phases, pending decisions,
tab histories, dirty revisions, capacity and host capabilities. Admission and
completed-operation retention have configurable count bounds. An evicted completed
operation reports `OperationExpired`; its existing handle still owns its result.
IDs belong to one fresh `RuntimeId` and must not be reused across runtimes.

## Guards and decisions

Register access guards with `App::entry_guard` or before starting `Router::run`.
Register disposal guards on `page.entry().on_dispose`. They receive owned
destination, identity, dirty state, revision and guard kind, and return `Allow`,
`Deny(reason)` or `Ask(DecisionRequest)`. Retaining an outgoing screen while
switching tabs or traversing history does not run its disposal guards.

Call `entry.set_dirty(true)` for every edit, including edits while already dirty.
Each call advances the revision and expires prior decisions. Without a custom
disposal guard, a dirty entry offers Discard and Cancel. Custom guards own their
save policy. A decision choice can register asynchronous `prepare` work, such as
saving and clearing dirty state. After it finishes, the router reruns guards if
the revision changed. All affected entries are revalidated before anything is
disposed. Mount failures leave the existing workspace intact.

The default UI calls `navigator.respond(decision_id, choice_id)`; custom presenters
use the same method and snapshot descriptors. IDs and allowed choices are checked.
An edit expires an old decision, while selecting or editing an unrelated tab does
not. Cancellation cannot undo a save or other external effects already performed
by application callbacks.

## Browser history

Use `.web(WebConfig::new().routing(WebRouting::Path))`, or `Hash` for fragment URLs.
Path deployments must serve the app shell for deep links. The example development
server provides that rewrite; missing asset requests still return 404.

Browser history records visible workspace visits independently of tab histories.
Visible open, selection, navigation and per-tab traversal push checkpoints;
initialization, replacement and selected-tab close replace the current checkpoint.
Hidden changes leave the URL alone. Closing the last tab clears Rake's current
checkpoint and shows the root URL.

Browser Back/Forward reuses matching live runtime/tab/entry hints, including
duplicate destinations. Stale hints fall back to normal guarded reveal or creation
from the checkpoint's complete destination. Restoration replaces state without
pushing a feedback visit. Newer traversals supersede older pending traversals;
denied, cancelled or failed traversal reconciles the URL to the committed screen.
History API failures are visible in `history_error` without rolling back a committed
tab operation.

Checkpoints are versioned destination records, not durable view or draft storage.
Reload reconstructs the URL destination in a fresh runtime. Browser termination
and cross-document navigation cannot promise asynchronous save completion.

## Current limits and evidence

There is one logical window. `NewWindow` reports `UnsupportedCapability` and the
snapshot advertises the supported targets. `TabMode` drives Foreground/Live activity;
see [managed work](managed-work.md). [Guarded application quit](guarded-quit.md),
[runtime control](runtime-control.md), [devtools](devtools.md) and
[development reload](development.md) use this same routing core. Durable workspace
persistence and native OS deep-link registration remain separate capabilities.

Routing tests use real destination codecs, local futures, operation scheduling,
guards and a recorded history sink. A separate resource test uses local stand-ins
to prove retention and mount-failure rollback. GPUI mounting and the actual History
API were additionally exercised in the native and browser examples; those smoke
checks are manual and currently cover macOS and Helium Chromium only.
