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

Quit invokes each entry's `on_dispose` guards with
`GuardKind::Dispose(DisposeReason::QuitApplication)`. A dirty entry without custom
guards offers Discard and Cancel. A custom guard owns its save policy and can
offer Save, Discard and Cancel through `DecisionRequest` and `DecisionChoice`.
Use `DecisionChoice::prepare` for asynchronous saving and report errors as
`GuardReason`. Rake cannot infer how an arbitrary application draft is saved.
Custom guards are responsible for protecting dirty state when installed.

Preparations can dispatch registered typed application commands through a weak
`Navigator` or `PageContext`. These commands use the same registry and operation
results as the CLI. Quit holds no tab scheduling lane while awaiting a save.
Capture weak entry/navigator handles or application models in guards, rather than
retaining the router/client inside its own entries or command registry.

Decisions retain their exact operation, entry and state revision. An edit expires
the old DecisionId. An edit after a response but before preparation also requires
a fresh decision without invoking that stale preparation. After asynchronous
preparation, Rake evaluates the updated revision again; a successful save should
mark the saved state clean. Failed saves and denied guards block quit. Cancel
preserves the workspace. No entries are partially disposed while later entries
still need approval. Completed saves are external effects and are not rolled back.

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

New routing requests fail with `OperationError::Quitting` while quit is pending
or committed. Application commands and finite tasks may still complete or register
dependent work before `Stopping`, including registered saves. They can extend the
wait; there is no implicit timeout or forced cancellation. A finite task or
registered command must return after submitting quit and leave observation to the
host: awaiting that quit from a job it is waiting to finish creates a self-dependency. The shell exposes job
counts, explicit job cancellation and Cancel quit. The Tasks/Operations panels
and CLI can also cancel individual work. Separately spawned work, blocking code
and external effects remain the application's responsibility.

`RouterSnapshot::quit`, `Router::quit_status()` and `Query::Quit` expose the phase,
operation ID, original policy and remaining finite work. Phases are `idle`,
`waiting_for_jobs`, `guarding`, `stopping`, `ready`, `cancelled` and `blocked`.
Cancellation is accepted before `Stopping`; after that point it returns false.
Cancellation takes priority over simultaneously ready work. Cancelling quit
reopens admission but cannot undo jobs already cancelled or completed saves.
Dropping an observer or disconnecting never cancels the operation.

Concurrent quit requests join the same operation; the first job policy wins.
After Cancel/Blocked, a new request starts a new operation. Identical request keys
always replay their original submission; use a fresh key to try again. Reload
and quit are mutually exclusive. A committed `QuitResult` identifies the runtime
whose workspace was disposed, not proof that its OS process has exited.

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

`quit` always reports submission immediately. Native exit may remove the transport
before a later status query; successful disconnection is not inferred as commit.
The native host flushes in-flight control responses before calling GPUI quit,
including the initial submission and final decision acknowledgement. Use
`quit status`, operation queries and Decisions while the runtime remains alive.
The Workspace devtools panel has both job policies and Cancel quit; Decisions
uses the same revision-checked save/discard/cancel responses.

## Native and Web boundaries

The default routed runner installs `rake::quit::RequestQuit`, a Quit menu and
Cmd-Q on macOS / Ctrl-Q on other native targets. Only `Ready` lets the runner call
GPUI quit. A native window's close button instead guards that host's retained
entries with `DisposeReason::CloseWindow`. Closing it cancels entry/tab work while
application-owned jobs and other windows continue. Closing the last host leaves the
runtime available for NewWindow or explicit Quit. Custom `Router::in_window` hosts
without a window adapter retain application-wide quit on close; they observe quit
readiness and own final host exit, as they own the driver and shell.

Without `rake`'s `native-lifecycle` feature, the pinned GPUI application shutdown
callback cannot veto an OS/Dock termination request. Those requests, forced
termination and `Router::shutdown()` are immediate cleanup boundaries, without a
guaranteed asynchronous save dialog. The `gpui` feature enables
`native-lifecycle`; on macOS it installs an AppKit termination bridge and routes
an ordinary Dock/Application terminate request through the same guarded quit
operation. The `updates` feature enables it transitively as well. Forced
termination remains outside the bridge's control.
Applications should route their own menu actions through `RequestQuit`/`Navigator::quit`
instead of calling GPUI quit directly.

On Web, a committed quit disposes the workspace and shows “Application closed.”
The document and development bridge remain available for status queries. It does
not attempt to close the browser window. Before a real browser close/reload or
cross-document navigation, `beforeunload` requests the browser's standard warning
when dirty entries, finite tasks or operations remain. Browsers control whether
that warning appears, often requiring prior user interaction. They cannot run
the asynchronous Rake decision flow during document destruction; no save or
durable-job guarantee follows from that warning. A supervised Web reload permits
its next unload only after saving the final compatible checkpoint; reload API
failure or an aborted boundary clears that permission. Ordinary browser closes
and reloads do not grant it.

## Verification

`cargo test -p rake --no-default-features --test guarded_quit` exercises public
portable behavior, including registered save commands, revisions, histories,
both job policies, cancellation, retries and reload exclusion. `cargo test -p
rake-cli --test runtime` includes real Unix-socket CLI decisions and quit status.
`node scripts/test-inspector-ui.mjs` checks actual panel code with DOM/transport
doubles. `python3 scripts/test-quit.py` is a separate macOS display test for real
native exit and pending HTTP-command coordination. Wasm compilation checks the
Web adapter; a real browser is required to verify its page lifecycle and warning.
