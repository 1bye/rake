# Development supervisor and reload

Build the CLI with `cargo build -p rake-cli` (or install it with
`cargo install --path crates/rake-cli`). The CLI bundles its Python helpers;
Python 3.11+ and the selected Rust/platform toolchain must be on PATH.

```sh
target/debug/rake doctor
target/debug/rake dev --target desktop --package rake-routing
target/debug/rake dev --target web --package rake-routing --port 8081
target/debug/rake inspect --session /tmp/rake-dev-EXAMPLE
target/debug/rake build --target desktop --package rake-routing --release
target/debug/rake build --target web --package rake-routing --release
target/debug/rake bundle --target desktop --package rake-routing
target/debug/rake bundle --target web --package rake-routing --base-path /notes/
```

Use `--manifest-path` outside the workspace root, `--bin` when there are multiple
binaries, `--features` for Cargo features, and `--web-dir` for an HTML/assets tree
(default: the package's `web` directory). Web needs a `cdylib`, the
`wasm32-unknown-unknown` target and a `wasm-bindgen` CLI matching Cargo.lock exactly.
Without an HTML entry point the builder generates a minimal loader. Build commands
write `dist/PACKAGE/TARGET`, or `--output PATH`; subsequent builds atomically replace
only outputs marked as belonging to this package and target. Failed builds preserve
the previous output. `bundle` adds signed native/resource bundles or static Web
deployment archives through this same build pipeline; see [packaging](packaging.md).
The development server is not a production Web server.

`dev` hashes local workspace/path-dependency source and asset contents, ignoring
`.git`, `target`, `dist`, `.rake`, `node_modules` and symlinked trees. Changes debounce
for 250 ms. One build runs at a time; changes during compilation coalesce, and a
superseded result is discarded before publication. Compilation happens while the
previous application remains usable. A compilation or binding-generation failure
keeps the previous native process/Web build. The watcher also remains alive after
an application crash. Editing a source restarts the build/launch cycle.

Each launch prints a private `/tmp/rake-dev-*` session directory. Its mode-0700
control directory isolates native instances and browser advertisements; checkpoints
and supervisor status use mode 0600. Inspect it with:

```sh
target/debug/rake dev status --session /tmp/rake-dev-EXAMPLE --json
RAKE_CONTROL_DIR=/tmp/rake-dev-EXAMPLE/control target/debug/rake instances --json
RAKE_CONTROL_DIR=/tmp/rake-dev-EXAMPLE/control target/debug/rake ctl reload status --json
```

Native dev opens the [graphical inspector](devtools.md) on a separate loopback
origin; Web dev opens the application with a Devtools link. Both print a private
launch URL. `--no-open` keeps the servers running without opening a browser.
The inspector stays available when an app crashes or restarts. `rake inspect`
also connects to already-running opt-in runtimes, using `RAKE_CONTROL_DIR` or
`--session`; it starts no application or build watcher. Use `--instance UUID`
to select an initial runtime when several are advertised.

`--json` emits supervisor events as JSON lines; compiler/application diagnostics go
to stderr. `status.json` remains readable when the app is stopped or crashed. Ctrl-C
or SIGTERM stops owned app, build, compiler, server and backend process groups and
removes their discovery files. Temporary session artifacts and a failed checkpoint
remain for inspection/recovery. Remove the session directory when it is no longer
needed; it may contain unsaved development data and large build artifacts.

## Shared runtime boundary

Routed and standalone root applications opt in with `ControlConfig::FromEnvironment`. Native dev sets
`RAKE_DEV_CONTROL=1` and a private control directory; Web uses the authenticated
same-origin development bridge. Ordinary builds enable neither adapter by default.
`App::root` keeps its `Services` mount signature; `App::root_with` receives a weak
`RuntimeContext` with the managed command, task, quit and update-restart boundary.
Standalone roots do not have router navigation or a checkpointed workspace. Their
reload is a fresh root replacement, and the supervisor first ensures the old host
reaches the managed runtime boundary before replacing it. If that host hand-off
cannot be coordinated, the old app remains usable; `--restart-without-state`
explicitly permits a process restart without restored state.

`App::background(...)` is native-only and starts the managed runtime without an
initial window; browser hosts reject it. Standalone window close uses
`ClosePolicy::Quit` by default, running quit guards and stopping managed work.
Choose `ClosePolicy::KeepRunning` explicitly when closing a window should leave the
application runtime and its background work alive.

The shared Rust/control/CLI surface provides:

```text
ctl reload status
ctl reload prepare BOUNDARY_ID [--mechanism native_restart|web_page_reload|native_code_replacement]
ctl reload checkpoint APPLICATION_ID
ctl reload abort BOUNDARY_ID
ctl reload apply BOUNDARY_ID
```

Preparation requires settled operations and decisions. Routed hosts stop navigation;
all hosts stop application-command and one-shot task admission, reject old
publications, cancel managed futures, and detach their managed root or route outlet
at a UI frame boundary. `ready` means managed futures have been dropped and every
window has acknowledged detachment (hidden hosts do not wait for a presentation
frame). A routed checkpoint is captured only then; standalone roots use fresh state
instead of inventing a router checkpoint.
Abort resumes eligible factories with fresh RunIds. One-shot tasks are cancelled
and are never automatically replayed. External saves and network effects are not
rolled back by this boundary.

A checkpoint contains application/envelope versions, source RuntimeId, GenerationId,
run high-water mark, all logical WindowIds/TabIds/EntryIds, per-window membership
and selection, routes, histories/cursors, selection ordering, modes, dirty revisions
and explicitly registered entry data. Version 2 captures multiple windows; version 1
single-window checkpoints remain readable. Restoration validates every host and
selection before mounting, creates physical replacement hosts, and attaches all
histories atomically. A failed mount retires provisional hosts. Closing the original
host does not prevent a later compatible restoration of the remaining windows.
OS window geometry is not part of the checkpoint.
The 512 KiB limit leaves room in the bounded control transport. Dirty entries without
a capture provider block reload instead of losing their changes.

## Opt-in entry state

`ReloadState` supplies versioned JSON defaults for owned data. Override `restore`
to support an explicit migration. Services remain available from `PageContext`
while mounting; reconstruct entities, HTTP clients and resources there.

```rust,ignore
#[rake::data]
struct DraftState { text: String }

impl rake::reload::ReloadState for DraftState {
    const SCHEMA: &'static str = "notes.draft";
    const VERSION: u32 = 1;
}

// Inside RouteViews::mount, before constructing the entry's new view:
let restored = page.entry().reload_state::<DraftState>()?;
// Initialize your owned draft model from `restored` or its normal default.
// Then register a synchronous closure capturing logical data (or a weak model):
page.entry().on_reload(move || current_draft.checkpoint())?;
```

See the routing example's `DraftState`: each independently retained draft registers
its own provider and consumes its own restored state. Holding strong UI/runtime
handles in the capture closure can form ownership cycles; use logical state or weak
handles. A replacement must consume any supplied entry checkpoint. Unknown routes,
schemas, versions, identities, application IDs, changed history limits, or failed
mounts reject the whole provisional workspace before attachment. Partial tabs never
appear. Failed state remains in storage and the shell offers an explicit empty
restart; a subsequent compatible build retries it. Reload checkpoints are separate
from durable sessions. Futures, callbacks, GPU objects and native pointers are never
serialized.

## Native adapter

The current default is explicitly reported as `native_restart`: build a replacement,
prepare/capture the old runtime, stop it, and launch the new binary with a private
checkpoint. A compatible restart receives a new RuntimeId, preserves logical tab and
entry IDs, increments GenerationId and starts eligible declared factories with fresh
RunIds. Pending operations and one-shot tasks do not survive the process.

The user's native hot reloader is still under development. To connect it, pass
`--native-reloader /absolute/path/to/backend`. After preparation/capture the supervisor
runs that executable with:

| Environment | Meaning |
| --- | --- |
| `RAKE_RELOAD_ARTIFACT` | Newly built native executable/artifact path |
| `RAKE_RELOAD_CHECKPOINT` | Private owned workspace JSON |
| `RAKE_RELOAD_RUNTIME` | Stable host RuntimeId |
| `RAKE_RELOAD_GENERATION` | Proposed next numeric generation |
| `RAKE_RELOAD_PROJECT` | Workspace directory |

The backend must exit zero only after supported code replacement succeeds. The
supervisor then applies the boundary and resumes eligible factories in the stable
host. Nonzero exit aborts the boundary and resumes old factories. The backend owns
ABI/layout validation, replacement and retirement; it must retain old code until
all objects, callbacks, futures and destructors requiring that code are gone.
Generation checks and Rake's managed-work quiescence alone **do not make native
unloading safe**. Rake currently supplies this integration contract, not a native
code loader. Successful backend replacement is therefore an assertion by that
backend, not something the supervisor can independently prove.

## Web adapter

The dev server starts before the first build; the waiting page automatically loads
the first successful result. It serves immutable per-build Wasm/bindings, so an old
page cannot mix new JavaScript with old Wasm. Public HTML/JavaScript contain no
session credential. Open the private launch URL printed by the server to authorize
that origin/tab; the bootstrap removes its authentication fragment before route
parsing and retains the credential in session storage.

A successful build asks each connected browser runtime to prepare through the same
lifecycle boundary. Busy saves/guard decisions keep the old app usable; CLI decision
responses continue to be forwarded between retries. The browser stores an owned
checkpoint in development-only session storage, captures once more immediately
before replacement, and reloads the entire page. The new page gets a new RuntimeId,
restores compatible logical state and replaces the current browser visit without
pushing another history entry. A failed checkpoint/storage operation aborts the
boundary and leaves the old app running. Restoration failures retain the original
checkpoint for a later build or explicit restart. The page displays build/reload
failures and reconnects to the broker independently of application diagnostics.
Static production builds have no bridge or reload polling.

Standalone `App::root` and `App::root_with` pages can also expose inspection through
the authenticated Web bridge. Their published-build reload uses full-page
replacement with fresh local state; no router checkpoint or `restored_from` lineage
is invented. Select the new runtime explicitly in an attached inspector. Routed
pages retain the checkpointed restoration behavior above. Controlled Wasm module
swapping is not implemented.
