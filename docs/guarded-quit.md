# Guarded application quit

Routed applications use one application-wide operation for the Quit button, native
Quit menu/shortcut, devtools and CLI. It scans every retained entry in every window,
including back and forward histories. Native window close and internal tab close
use narrower disposal operations; see [native windows](routing.md#native-windows).

```rust
use rake::prelude::*;
use rake::router::NoRoute;

let router = Router::<NoRoute>::new(RouterConfig::default());
let operation = router.navigator().quit(QuitOptions { jobs: QuitJobs::Wait });
assert_eq!(router.quit_status().operation_id, Some(operation.id()));
// The host owns router.run(). operation.wait() only observes its outcome.
```

## Guards and decisions

Quit invokes each entry's `on_dispose` guards with `GuardKind::Dispose(DisposeReason::QuitApplication)`. A dirty entry without custom guards offers Discard and Cancel.

Rake can't infer how to save an arbitrary draft, so a custom guard owns its save policy and is responsible for protecting dirty state. It can offer Save, Discard and Cancel through `DecisionRequest` and `DecisionChoice`. Use `DecisionChoice::prepare` for asynchronous saving, and report errors as `GuardReason`.

Preparations can dispatch registered typed commands through a weak `Navigator` or `PageContext`, using the same registry and results as the CLI. Quit holds no tab scheduling lane while awaiting a save.

In guards, capture weak entry or navigator handles or application models, rather than retaining the router or client inside its own entries or command registry.

Decisions retain their exact operation, entry and state revision.

- An edit expires the old `DecisionId`.
- An edit after a response but before preparation requires a fresh decision, without running the stale preparation.
- After asynchronous preparation, Rake evaluates the updated revision again, so a successful save should mark the state clean.
- Failed saves and denied guards block quit. Cancel preserves the workspace.
- No entries are partially disposed while later entries still need approval.
- Completed saves are external effects and aren't rolled back.

For state not represented by a retained route entry, register `Router::quit_guard` before starting `Router::run`. These callbacks run after finite work and entry approvals, immediately before `Stopping`. `QuitGuardContext` supplies the operation ID and selected `QuitOptions`.

Return `Err(GuardReason(...))` to block quit. The callback owns its state and must re-read it before returning `Ok(())`: Rake rechecks retained-entry revisions afterwards, but can't revalidate arbitrary external state. Cancelling the quit while a callback is pending uses the normal outer cancellation path and preserves the workspace.

## Running work and cancellation

`QuitOptions::jobs` selects an explicit policy:

| Policy | Existing routing/application operations | Finite tab/application tasks |
| --- | --- | --- |
| `Wait` (default) | Settle before collecting entry approvals | Finish after approvals, then recheck all approved revisions |
| `Cancel` | Request cancellation before scanning entries | Request cancellation only after all entry guards approve |

Activity and while-visible factories are not finite jobs. They continue while
quit is awaiting approval and stop at the irreversible boundary. Successful quit
waits until managed futures are dropped, closes scopes and disposes all entries.
Queued publications drain before final revision validation. Late job edits force
new guards instead of silently discarding the changed revision.

New routing requests fail with `OperationError::Quitting` while quit is pending or committed. Commands and finite tasks may still complete or register dependent work before `Stopping`, including saves. They can extend the wait, because there's no implicit timeout or forced cancellation.

> A finite task or registered command must return after submitting quit and leave observation to the host. Awaiting that quit from a job it's waiting on creates a self-dependency.

The shell exposes job counts, explicit job cancellation and Cancel quit. The Tasks and Operations panels and the CLI can also cancel individual work. Separately spawned work, blocking code and external effects remain the application's responsibility.

`RouterSnapshot::quit`, `Router::quit_status()` and `Query::Quit` expose the phase, operation ID, original policy and remaining finite work. Phases are `idle`, `waiting_for_jobs`, `guarding`, `stopping`, `ready`, `cancelled` and `blocked`.

- Cancellation is accepted before `Stopping`, and returns false after it.
- Cancellation takes priority over simultaneously ready work.
- Cancelling quit reopens admission, but can't undo jobs already cancelled or completed saves.
- Dropping an observer or disconnecting never cancels the operation.

Concurrent quit requests join the same operation, and the first job policy wins. After Cancel or Blocked, a new request starts a new operation. Identical request keys always replay their original submission, so use a fresh key to try again.

Reload and quit are mutually exclusive. A committed `QuitResult` identifies the runtime whose workspace was disposed; it doesn't prove the OS process has exited.

## CLI and devtools

```sh
rake ctl quit --request-key quit-1 --json
rake ctl quit status --json
rake ctl decisions list --json
rake ctl decisions respond decision-7 --choice save --json
rake ctl operations cancel op-3 --json
# A new attempt with explicit job cancellation:
rake ctl quit --jobs cancel --request-key quit-2 --json
```

`quit` always reports submission immediately. Native exit may remove the transport before a later status query, and successful disconnection isn't inferred as commit. The native host flushes in-flight control responses before calling GPUI quit, including the initial submission and final decision acknowledgement.

Use `quit status`, operation queries and Decisions while the runtime is alive. The Workspace DevTools panel has both job policies and Cancel quit, and Decisions uses the same revision-checked save, discard and cancel responses.

## Native and Web boundaries

The default routed runner installs `rake::quit::RequestQuit`, a Quit menu and Cmd-Q on macOS or Ctrl-Q on other native targets. Only `Ready` lets the runner call GPUI quit.

A native window's close button instead guards that host's retained entries with `DisposeReason::CloseWindow`. Closing it cancels entry and tab work, while application-owned jobs and other windows continue. Closing the last host leaves the runtime available for NewWindow or explicit Quit.

Custom `Router::in_window` hosts without a window adapter keep application-wide quit on close. They observe quit readiness and own final host exit, as they own the driver and shell.

Without `rake`'s `native-lifecycle` feature, GPUI's shutdown callback can't veto an OS or Dock termination request. Those requests, forced termination and `Router::shutdown()` are immediate cleanup boundaries with no guaranteed asynchronous save dialog.

The `gpui` and `updates` features both enable `native-lifecycle`. On macOS it installs an AppKit termination bridge and routes an ordinary Dock or Application terminate request through the same guarded quit. Forced termination stays outside its control.

Route your own menu actions through `RequestQuit` or `Navigator::quit` instead of calling GPUI quit directly.

On Web, a committed quit disposes the workspace and shows “Application closed.” The document and development bridge remain available for status queries, and Rake doesn't try to close the browser window.

Before a real browser close, reload or cross-document navigation, `beforeunload` requests the browser's standard warning when dirty entries, finite tasks or operations remain, or when an application-wide quit guard is registered.

- Browsers control whether the warning appears, often requiring prior user interaction.
- They can't run Rake's asynchronous decision flow during document destruction, so the warning guarantees no save or durable job.
- A supervised Web reload permits its next unload only after saving the final compatible checkpoint. A reload API failure or aborted boundary clears that permission. Ordinary browser closes and reloads don't grant it.

## Verification

A real browser is required to verify the Web page lifecycle and warning. See [test coverage](internal/test-coverage.md) for what's automated.
