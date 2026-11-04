# Managed work

The router drives local tasks and restartable entry activity on the native or Web
UI event loop. Futures do not require `Send`. Async I/O yields to the host; CPU or
blocking work needs a separate worker facility, which is not implemented yet.

## Ownership and eligibility

| Work | Hidden page | Entry navigation | Tab close |
| --- | --- | --- | --- |
| `page.activity(key, factory)` | Foreground stops; Live continues | Outgoing run stops | Stops and unregisters |
| `page.while_visible(key, factory)` | Stops in either mode | Outgoing run stops | Stops and unregisters |
| `page.tab().tasks().spawn(label, future)` | Continues | Continues | Cancellation requested |
| `page.app().tasks().spawn(label, future)` | Continues | Continues | Continues |

- **Foreground** requires the entry to be current and selected for presentation, and its host visible.
- **Live** keeps only a tab's current entry eligible. Historical entries stay retained but stopped.
- **`while_visible`** always requires presentation and host visibility, including in Live tabs.

Changing mode recalculates eligibility without restarting an already eligible run. Keyboard focus isn't a visibility signal.

An activity registers one factory per nonempty resource key in its entry.

- Each transition from stopped to active invokes the factory, creating a new future with a fresh `RunId`.
- Completed or failed runs stay finished while eligibility is unchanged.
- Returning to the page starts a fresh run, not a resumed future.
- Cancelling its `TaskHandle` also unregisters the factory.
- Scope disposal unregisters all its factories and releases their captures.

Entry changes and disposal take effect when navigation commits. A pending or
cancelled disposal guard leaves work eligible. Mount failures do not start the
uncommitted entry's activity or stop the committed screen. Entry lifetime
cancellation and run cancellation are separate: retained history entries remain
alive even though their activity stopped.

## Using the API

### View-bound loading

`runtime.load(cx, factory)` starts an arbitrary asynchronous value and redraws
its owning GPUI view automatically. `page.load(cx, factory)` uses the retained
entry's finite task scope, so hiding the page does not cancel it. Dropping the
loader or disposing its owner cancels the attempt. Neither results nor errors
need `Clone` or `Send`.

```rust,no_run
use rake::prelude::*;
struct Screen;
fn bind(runtime: &RuntimeContext, cx: &mut rake::gpui::Context<Screen>)
    -> Loader<String, std::io::Error>
{
    runtime.load(cx, |load| async move {
        load.background_executor()
            .spawn(async { std::fs::read_to_string("welcome.txt") })
            .await
    })
}
```

The factory receives `LoadContext` with shared `services()`, `http()`,
`cancellation()`, and `background_executor()`. Async I/O runs on the UI loop;
blocking/CPU work must use a worker. A separately spawned worker may keep
running after cancellation, but cannot replace a newer result.

Read `loader.state(cx)` in `Render`, call `loader.retry(cx)` to start a fresh
attempt, or `loader.cancel(cx)` to stop. Failures distinguish typed
`LoadError::Source`, managed-work `LoadError::Task`, and `LoadError::Cancelled`.
Task admission failures are visible immediately. Rake retains subscriptions and
terminal observation internally; applications do not register completion events.

The [foundation example](../examples/foundation/src/lib.rs) shows this API with
`rake::view!`. Its configured HTTP endpoint must be served for the request to
succeed; a connection failure is shown with a retry button.

The [routing example](../examples/routing/src/lib.rs) registers refresh and preview
clocks and displays their counters and run identities. Its Foreground/Live buttons
change policy; tab download and app export buttons exercise longer-lived scopes.
Exports remain visible from another screen after their originating tab closes.

```rust
page.on_event(move |loaded: ProjectLoaded| {
    // Capture a weak GPUI entity and owned AsyncAppContext, then update on the UI loop.
    apply_project(loaded)
})?;

page.activity("refresh", move |run| {
    let http = http.clone();
    async move {
        let project = http.get("project")
            .cancellation(run.cancellation())
            .send_json().await?;
        run.publish(ProjectLoaded(project))
    }
})?;
```

The snippet is schematic; the executable example supplies the typed event and
GPUI handler. `PageContext` and the headless `EntryHandle` expose the same activity,
visibility-activity and typed-handler methods. `Router::app()` provides an app task
scope to services. `Navigator::app()` preserves a page's causal origin.

### Generation-safe loading state

`LoadState<T, E>` is a small synchronous state machine for view/model loading. It
does not start a request, choose an executor, cache data or replace managed work.
Start the actual request in an application, tab or activity task, keep the token,
and complete only that generation:

```rust,no_run
use rake::prelude::{LoadState, LoadStatus};

struct Project;
let project = Project;
let mut load = LoadState::<Project, String>::default();
let token = load.begin();
// An owned task performs the HTTP request and later calls:
let accepted = load.complete(token, Ok(project));
assert!(accepted);
assert!(matches!(load.state(), LoadStatus::Ready(_)));
```

Calling `begin` again or `reset` makes an older token stale; its late result is
rejected and cannot overwrite the current state. Use `LoadStatus::Loading` to
render progress and `LoadStatus::Failed` to render a retry affordance. The task
that owns the request still needs cancellation and lifetime policy from
`ActivityRun`, `TaskScope` or an application-specific model.

- `spawn` accepts one `Future<Output = Result<(), TaskError>>`.
- `spawn_with` accepts one factory invocation receiving an `ActivityRun`, useful for cancellation and checked publication in tab or app tasks.

Unlike activity factories, these jobs never restart automatically. Submission is eager, so observing or dropping the handle doesn't control execution. `TaskHandle::wait` observes the latest run's terminal `TaskState`, and failures stay inspectable as `Failed(TaskError)`. `TaskError` converts from Rake HTTP errors, and domain failures can use `Failed(String)`.

## Publication and cancellation

`ActivityRun::publish(event)` queues delivery to the scope's registered typed
handler. One handler per event type is supported; registering another replaces it.
Handlers execute from the runtime's UI driver without a borrowed GPUI context
crossing an await. They return `Result<(), TaskError>`; handler failure stops the
originating run and becomes an observable task failure.

Publication checks owner lifetime, run acceptance, cancellation and generation. Delivery checks again, so an event queued just before hiding, navigation, restart or close can't mutate a later run or disposed owner.

- A successful publication means admitted, not already applied.
- A completed future's already queued events are still delivered, but it can't publish new ones from a saved context.
- Tab-owned publications survive selection and entry changes.
- Application work keeps its original runtime, window, tab and entry IDs after its source closes. The application decides what to do with such results.

Cancellation invalidates results immediately, wakes the driver, and drops the future on its next poll. Use `run.cancellation()` with HTTP or application work; `CancellationToken::cancelled()` supports cooperative waits.

Cancelling async work doesn't roll back external effects or stop separately spawned or blocking work. Owner disposal and driver teardown reject new work and cancel outstanding jobs. Application tasks end with their runtime, even if every internal tab has closed.

## Host behavior and observation

Web uses `document.hidden`, `visibilitychange`, `pagehide` and `pageshow`, with listeners owned and removed by the host adapter. Foreground stops while the document is hidden, and Live stays eligible.

Browsers can throttle, freeze or discard the whole document, so eligibility doesn't promise uninterrupted execution or a durable background service. Restoring a page from browser cache rechecks visibility. Actual document destruction can't guarantee asynchronous cleanup.

The owned Web router/control driver wakes through the browser's microtask
executor, avoiding GPUI's chained-timer dispatcher for runtime progress in hidden
tabs. Dropping the workspace aborts that driver. Application timers still follow
browser scheduling and suspension policies.

How each platform observes visibility:

| Platform | Observation |
| --- | --- |
| macOS | Reads AppKit window visibility, minimization and occlusion on the UI loop every 250 ms |
| Windows | Observes visible, minimized and DWM-cloaked state, but not overlap by other windows |
| Linux | Reports `Unknown`, which is treated as visible |

Keyboard focus is never used as a signal. Custom hosts can supply observations with `Router::set_window_visibility(id, visibility)`. `set_host_visibility` is a convenience targeting the initial host.

Every native window tracks visibility and selection independently, so hiding or closing one doesn't suspend another's selected entry. Closing a window cancels its entry and tab scopes, while application-owned jobs survive, even after the last host closes. Task origins keep the originating `WindowId`, and window snapshots expose the observed visibility.

[Guarded quit](guarded-quit.md) coordinates finite jobs and disposal guards before stopping activity factories. Forced native termination requests cancellation before host teardown.

`RouterSnapshot::work` reports owners, causal origins, resource keys, run and task identities, accepted generation, eligibility, outcomes and pending publications. `Router::changed` covers work updates as well as navigation. Tracing records task, run, generation and causal IDs separately from the snapshot.

`RouterConfig::tasks` sets registration and record capacity, terminal retention and publication queue bounds.

- Admission or publication overflow returns an error.
- Expired terminal handles report `TaskError::Expired`.
- Active factory registrations stay retained, and rapid eligibility changes coalesce unstarted runs per registration.

Reload boundaries advance accepted generations; see [development](development.md).
[Runtime control](runtime-control.md) and [guarded quit](guarded-quit.md) share
task/operation cancellation and revision-checked decisions. Automatic retry, worker
threads and durable jobs remain separate capabilities. OS or browser termination
cannot guarantee a custom asynchronous save dialog.
