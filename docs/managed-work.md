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

Foreground requires the entry to be current, selected for presentation, and its
host visible. Live keeps only a tab's current entry eligible; historical entries
remain retained but stopped. `while_visible` always requires presentation and host
visibility, including in Live tabs. Changing mode recalculates eligibility without
restarting an already eligible run. Keyboard focus is not a visibility signal.

An activity registers one factory per nonempty resource key in its entry. Each
eligibility transition from stopped to active invokes the factory to construct a
new future and assigns a fresh `RunId`. Completed or failed runs stay finished
while eligibility remains unchanged. Returning to the page starts a fresh run,
not a resumed future. Explicitly cancelling its `TaskHandle` also unregisters the
factory. Scope disposal unregisters all its factories and releases their captures.

Entry changes and disposal take effect when navigation commits. A pending or
cancelled disposal guard leaves work eligible. Mount failures do not start the
uncommitted entry's activity or stop the committed screen. Entry lifetime
cancellation and run cancellation are separate: retained history entries remain
alive even though their activity stopped.

## Using the API

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

`spawn` accepts one `Future<Output = Result<(), TaskError>>`. `spawn_with` accepts
one factory invocation receiving an `ActivityRun`, useful for cancellation and
checked publication in tab or app tasks. Unlike activity factories, these jobs
never restart automatically. Submission is eager; observing or dropping the
returned handle does not control execution. `TaskHandle::wait` observes the latest
run's terminal `TaskState`; failures remain inspectable as `Failed(TaskError)`.
`TaskError` converts from Rake HTTP errors; domain failures can use `Failed(String)`.

## Publication and cancellation

`ActivityRun::publish(event)` queues delivery to the scope's registered typed
handler. One handler per event type is supported; registering another replaces it.
Handlers execute from the runtime's UI driver without a borrowed GPUI context
crossing an await. They return `Result<(), TaskError>`; handler failure stops the
originating run and becomes an observable task failure.

Publication checks owner lifetime, run acceptance, cancellation and generation.
Delivery checks again, so an event queued just before hiding, navigation, restart
or close cannot mutate a later run or disposed owner. A successful publication
means admitted, not already applied. Successful future completion allows its
already queued events to be delivered; it does not permit new publications from
a saved context. Tab-owned publications survive selection and entry changes.
Application work retains original runtime/window/tab/entry IDs after its source
closes. The application owns the model policy for such results.

Cancellation invalidates results immediately, wakes the driver, and drops the
future on its next poll. Use `run.cancellation()` with HTTP or application work;
`CancellationToken::cancelled()` supports cooperative waits. Cancelling async work
does not roll back external effects or stop separately spawned/blocking work.
Owner disposal and driver teardown reject new work and cancel outstanding jobs.
Application tasks end with their runtime, even if every internal tab has closed.

## Host behavior and observation

Web uses `document.hidden`, `visibilitychange`, `pagehide` and `pageshow`, with
listeners owned and removed by the host adapter. Foreground stops while the
document is hidden; Live remains eligible. Browsers can throttle, freeze or discard
the entire document. Eligibility does not promise uninterrupted execution or a
durable background service. Page restoration from browser cache rechecks visibility;
actual document destruction cannot guarantee asynchronous cleanup.

The owned Web router/control driver wakes through the browser's microtask
executor, avoiding GPUI's chained-timer dispatcher for runtime progress in hidden
tabs. Dropping the workspace aborts that driver. Application timers still follow
browser scheduling and suspension policies.

The macOS adapter reads AppKit window visibility, minimization and occlusion on
the UI loop every 250 ms. This bounds observation frequency without confusing
keyboard focus with presentation. [Guarded quit](guarded-quit.md) coordinates
finite jobs and disposal guards before stopping activity factories. Forced native
termination requests cancellation before host teardown. Other native backends currently report `Unknown`; unknown
visibility assumes visible, and custom hosts can supply actual observations with
`Router::set_host_visibility`. Snapshots expose the observed value explicitly.

`RouterSnapshot::work` reports owners, causal origins, resource keys, run and task
identities, accepted generation, eligibility, outcomes and pending publications.
`Router::changed` covers work updates as well as navigation. Tracing records task,
run, generation and causal IDs independently of the authoritative snapshot.
`RouterConfig::tasks` sets registration/record capacity, terminal retention and
publication queue bounds. Admission or publication overflow returns an error.
Expired terminal handles report `TaskError::Expired`. Active factory registrations
stay retained; rapid eligibility changes coalesce unstarted runs per registration.

Reload boundaries advance accepted generations; see [development](development.md).
[Runtime control](runtime-control.md) and [guarded quit](guarded-quit.md) share
task/operation cancellation and revision-checked decisions. Automatic retry, worker
threads and durable jobs remain separate capabilities. OS or browser termination
cannot guarantee a custom asynchronous save dialog.
