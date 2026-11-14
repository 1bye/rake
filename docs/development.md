# Development supervisor and reload

Build the CLI with `cargo build -p rake-app-cli` (or install it with
`cargo install --path crates/rake-cli`). The CLI runs the development toolchain in Rust. Put the selected Rust/platform
toolchain on PATH; no Python interpreter is required. Repository-only Python
verification scripts still require Python 3.11+.

On Windows, `rake doctor`, `rake build`, native/Web `rake bundle` and the Node
agent client are supported. Development supervision and native local inspection
(`dev`, `inspect`, `ctl` and `cdp`) currently require Unix. The remaining Windows
IPC, discovery-security and supervisor work is recorded in
[PORTING.md](../crates/rake-cli/PORTING.md). macOS native preview keeps its Swift
capture helper and needs the Xcode tools and capture permissions.


```sh
target/debug/rake doctor
target/debug/rake dev --target desktop --package rake-routing
target/debug/rake dev --target web --package rake-routing --port 8081
target/debug/rake cdp --session /tmp/rake-dev-EXAMPLE --no-open
target/debug/rake build --target desktop --package rake-routing --release
target/debug/rake build --target web --package rake-routing --release
target/debug/rake bundle --target desktop --package rake-routing
target/debug/rake bundle --target web --package rake-routing --base-path /notes/
```

`doctor --json` reports all installed Rust targets as `installed_targets` and
includes Node's version as `node`. Missing Node is informational and does not
affect readiness; web readiness requires the WASM target and `wasm-bindgen`.

Options:

- `--manifest-path` when outside the workspace root.
- `--bin` when there are multiple binaries.
- `--features` for Cargo features.
- `--web-dir` for an HTML and assets tree (default: the package's `web` directory).

Web needs a `cdylib`, the `wasm32-unknown-unknown` target and a `wasm-bindgen` CLI matching `Cargo.lock` exactly. Without an HTML entry point, the builder generates a minimal loader.

Build commands write to `dist/PACKAGE/TARGET`, or `--output PATH`. Later builds atomically replace only outputs belonging to this package and target, and failed builds keep the previous output. `bundle` adds signed native or resource bundles and static Web archives through the same pipeline; see [packaging](packaging.md). The development server isn't a production Web server.

`dev` hashes local workspace and path-dependency sources and assets, ignoring `.git`, `target`, `dist`, `.rake`, `node_modules` and symlinked trees. Changes debounce for 250 ms.

- One build runs at a time. Changes during compilation coalesce, and a superseded result is discarded before publication.
- The previous app stays usable while compiling, and a compile or binding failure keeps it.
- The watcher stays alive after an app crash. Editing a source restarts the build and launch cycle.

Development output shows the Rake CLI version, the app's bundle display name (or
package name), startup time and editing behavior. Ordinary native saves report
`Restarted`; live edits report `Updated` only after the running app acknowledges
a patch. A compiled edit without an acknowledgment reports `Update ready`.
Web publication also reports
`Update ready`, since publishing a build does not confirm that a browser reloaded.
Source paths and timings accompany completed edits. Compiler warnings and errors
remain visible, with routine tool progress hidden. Application output is prefixed
with `app`; it is not synthesized by the supervisor. Known view resets are reported
before restart, without claiming that every unregistered field can be detected.

Use `--verbose` for full tool output, internal supervisor events and the private
session directory. Styling is automatic for terminals, using the terminal's colors;
`NO_COLOR` disables color, and `TERM=dumb` disables terminal progress and Unicode
markers. Redirected output has no color or cursor updates. Modern Windows terminals
and native Windows console colors are supported; older consoles use ASCII markers.
Interactive builds show the current activity and elapsed time, including project,
live compiler and preview preparation. An owned progress thread sleeps between
updates and parks while idle; app logs and progress share the same writer lock.
Plain output prints each activity once. At most 1 MiB of human build diagnostics
is retained until the result;
an explicit notice reports additional output, available live with `--verbose`.

`--verbose` prints the private `/tmp/rake-dev-*` session directory (the system's
private temporary directory on Windows). Its mode-0700
control directory isolates native instances and browser advertisements; checkpoints
and supervisor status use mode 0600. Inspect it with:

```sh
target/debug/rake dev status --session /tmp/rake-dev-EXAMPLE --json
RAKE_CONTROL_DIR=/tmp/rake-dev-EXAMPLE/control target/debug/rake instances --json
RAKE_CONTROL_DIR=/tmp/rake-dev-EXAMPLE/control target/debug/rake ctl reload status --json
```

Desktop dev launches the native app; `--preview` prints its browser preview URL
as soon as the server is ready. Click or copy the link to open it when you choose.
Starting with CLI `0.1.0-alpha.5`, `rake dev --live --preview` works on Apple Silicon
Macs: preview follows live patches and reconnects after checkpointed restarts.
Web dev opens the app directly. With `--cdp`, it opens the embedded
[Chrome DevTools](devtools.md) toggle instead. Node 22+ is required for Web
`--cdp`; frontend assets download when the panel opens. `--no-open` prints the
private launch URL without opening a browser. Build and reload status is also
available through `rake dev status --session PATH`.

Use `rake dev --cdp` to start the Chrome DevTools adapter with the app, or
`rake inspect --session PATH` / `rake cdp --session PATH` to attach to an existing
session. `--instance UUID` selects a runtime. The legacy HTML and GPUI inspectors
are disabled and are not bundled.

`--json` emits the existing supervisor events as JSON lines, and compiler and app diagnostics go to stderr. `status.json` stays readable when the app is stopped or crashed.

Ctrl-C or SIGTERM stops owned app, build, compiler, server and backend process groups and removes their discovery files. Temporary session artifacts and a failed checkpoint remain for recovery. Remove the session directory when you're done, because it may contain unsaved development data and large build artifacts.

## Shared runtime boundary

Routed and standalone root apps opt in with `ControlConfig::FromEnvironment`. Native dev sets `RAKE_DEV_CONTROL=1` and a private control directory, and Web uses the authenticated same-origin development bridge. Ordinary builds enable neither.

`App::root` keeps its `Services` mount signature. `App::root_with` receives a weak `RuntimeContext` with the managed command, task, quit and update-restart boundary.

Standalone roots have no checkpointed workspace: a restart creates a fresh root.
With Blaze, the supervisor restarts and reports that state was not preserved.
`--require-state` (or `RAKE_REQUIRE_STATE=1`) instead keeps the old process alive
when state cannot be preserved. An unavailable/busy runtime boundary still waits
or reports its error; the supervisor does not discard an in-flight operation.
Plain Cargo supervision retains its strict checkpoint capture and requires
`--restart-without-state` to restart an app without a control endpoint.

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

Preparation requires settled operations and decisions. Routed hosts stop navigation. All hosts stop command and one-shot task admission, reject old publications, cancel managed futures, and detach their managed root or route outlet at a UI frame boundary.

`ready` means managed futures have been dropped and every window has acknowledged detachment (hidden hosts don't wait for a frame). A routed checkpoint is captured only then, and standalone roots use fresh state.

Abort resumes eligible factories with fresh RunIds. One-shot tasks are cancelled and never replayed. External saves and network effects aren't rolled back.

A checkpoint contains application and envelope versions, source RuntimeId, GenerationId, run high-water mark, all logical window, tab and entry IDs, per-window membership and selection, routes, histories and cursors, selection ordering, modes, dirty revisions and explicitly registered entry data. Version 2 captures multiple windows, and version 1 single-window checkpoints stay readable.

- Restoration validates every host and selection before mounting, creates replacement hosts and attaches all histories atomically. A failed mount retires provisional hosts.
- Closing the original host doesn't prevent later compatible restoration of the remaining windows.
- OS window geometry isn't included.
- The 512 KiB limit leaves room in the bounded control transport.
- Dirty entries without a capture provider block reload instead of losing changes.

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

See the routing example's `DraftState`: each retained draft registers its own provider and consumes its own restored state. Strong UI or runtime handles in the capture closure can form ownership cycles, so use logical state or weak handles. A replacement must consume any supplied entry checkpoint.

Unknown routes, schemas, versions, identities, application IDs, changed history limits or failed mounts reject the whole provisional workspace before attachment, so partial tabs never appear. Failed state stays in storage, the shell offers an explicit empty restart, and a later compatible build retries it.

Reload checkpoints are separate from durable sessions. Futures, callbacks, GPU objects and native pointers are never serialized.

## Native adapter

The current default is reported as `native_restart`: build a replacement, prepare and capture the old runtime, stop it, and launch the new binary with a private checkpoint.

A compatible restart gets a new RuntimeId, keeps logical tab and entry IDs, increments GenerationId and starts eligible declared factories with fresh RunIds. Pending operations and one-shot tasks don't survive the process.

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

The backend must exit zero only after supported code replacement succeeds. The supervisor then applies the boundary and resumes eligible factories in the stable host. A nonzero exit aborts and resumes old factories.

The backend owns ABI and layout validation, replacement and retirement, and must keep old code until all objects, callbacks, futures and destructors needing it are gone.

> Generation checks and Rake's managed-work quiescence alone **don't make native unloading safe**. Rake supplies this integration contract, not a native code loader, and can't independently prove a backend's replacement succeeded.

## Web adapter

The dev server starts before the first build, and its waiting page loads the first successful result. It serves immutable per-build Wasm and bindings, so an old page can't mix new JavaScript with old Wasm.

Public HTML and JavaScript contain no session credential. Open the private launch URL to authorize that origin and tab. The bootstrap removes its authentication fragment before route parsing and keeps the credential in session storage.

A successful build asks each connected browser runtime to prepare through the same lifecycle boundary. Busy saves and guard decisions keep the old app usable, and CLI decision responses are still forwarded between retries.

1. The browser stores an owned checkpoint in development-only session storage.
2. It captures once more immediately before replacement and reloads the whole page.
3. The new page gets a new RuntimeId, restores compatible logical state and replaces the current browser visit without adding a history entry.

A failed checkpoint or storage operation aborts and leaves the old app running. Restoration failures keep the original checkpoint for a later build or explicit restart. The page shows build and reload failures and reconnects to the broker independently of app diagnostics. Static production builds have no bridge or reload polling.

Standalone `App::root` and `App::root_with` pages can also expose inspection through the authenticated Web bridge. Their reload is a full-page replacement with fresh local state, with no router checkpoint, so select the new runtime explicitly in an attached inspector. Controlled Wasm module swapping isn't implemented.


## Experimental pinned native live reload (Apple Silicon)

Use a CLI built from this branch. From the workspace root, a local invocation is
`cargo run -p rake-app-cli -- dev --live --manifest-path examples/view/Cargo.toml --bin rake-view`.

`rake dev --live` uses the Rake-patched Blaze compiler, independently of a `blaze`
on `PATH`. It invokes the pinned engine directly, so a workspace's Blaze version
pin cannot redirect live mode to another compiler. The first run downloads the exact archive in
`crates/rake-cli/blaze-pin.toml`, checks its SHA-256 before extracting it, and
installs it in `~/.rake/blaze/<version>/`. Later starts recheck the saved archive
and installed files. A wrong checksum or damaged install is an error.

```sh
cd examples/view
rake dev --live --bin rake-view
rake doctor --json
```

This is currently **Apple Silicon macOS only**. The pinned
[`v0.20261030.1-rake.3` release](https://github.com/1bye/blaze/releases/tag/v0.20261030.1-rake.3)
is published; ordinary live mode downloads it automatically. This version loads
manifests introduced by launch-only dependency features, including `rake/devtools`
under `--cdp`. An app does not need a `devtools` forwarding feature or a manual
`hdrhistogram` lockfile entry to work around the older planner bug. To use a copy
of that exact published archive offline:

```sh
BLAZE_ARCHIVE=/absolute/path/to/blaze-v0.20261030.1-rake.3-aarch64-apple-darwin.tar.gz rake dev --live --bin rake-view
```

CLI `0.1.0-alpha.6` embeds rake.3; published CLI alpha.5 retains rake.2.
Upgrade the installed CLI to receive the compiler fix. Updating an app's SDK
alone does not change the compiler pin.

`BLAZE_ARCHIVE` supplies bytes, not a checksum bypass. `RAKE_BLAZE=/absolute/path/to/blaze`
is an explicit developer override; it requires the patched
`rust-analyzer-proc-macro-srv` beside that executable and bypasses the release pin.
Doctor reports the pinned installation's platform, version, checksum and server
readiness without installing anything.

For an app outside this workspace, expose Rake's live dependency feature in the
app manifest and refresh the lockfile before the first live build:

```toml
[features]
live = ["rake/live"]
```

```sh
cargo metadata --format-version 1 > /dev/null
rake dev --live --bin your-app
```

Use your dependency alias in place of `rake` if it is renamed. This includes the
optional `rake-blaze-live` registry runtime in the app's normal lockfile. The
repository's workspace examples already expose the live feature.

Live mode enables the app's existing Rake `live` feature, which automatically
enables `pathfinder_simd/pf-no-simd` on macOS. It sets the hybrid macro options
`BLAZE_PROC_MACRO_WASM=0`, `BLAZE_PROC_MACRO_SERVER=0`,
`LIBLZMA_NO_PKG_CONFIG=1`, sandboxing, and the 64 MiB Rust stack. The compiler
build command always selects `--bin`; libraries such as an app's `cdylib` are
not built for this mode. The packaged fixed macro server is selected explicitly;
no toolchain binary is replaced. Runtime/store directories live beside the
versioned compiler. The pinned Rust toolchain may download on the first build.

The supervisor sends ordinary saves directly to the daemon. The app polls on
its UI thread every 16 ms and redraws after a patch, including occluded windows.
Layout changes or patch-area exhaustion use the existing checkpointed restart;
apps must register the state they want restored. Configure the app with
`.control(rake::control::ControlConfig::FromEnvironment)` so the supervisor can
observe outcomes and coordinate a restart. The endpoint is enabled only in the
development environment. On a layout change, the default is to restart: registered
entry checkpoints restore, and entries without a checkpoint reset. Standalone
roots restart fresh. Before restarting, the CLI names the reset entries (route
name and entry ID); the replacement shows the same non-interactive status for
six seconds without taking focus. Fully checkpointed restarts show no warning.
Use `--require-state` or `RAKE_REQUIRE_STATE=1` to block missing-checkpoint
restarts and keep the old process and state in Blaze mode. Omit the flag and
unset `RAKE_REQUIRE_STATE` for Blaze's permissive default. Cargo retains its
existing `--restart-without-state` opt-in and strict checkpoint policy. The pinned Blaze engine latches a type-change restart
request for the lifetime of the running process. In strict mode, later saves
are still handled, but reverting the field does not resume body patches: they
can continue requesting a restart. Preserve or save the old state before
restarting the session; the supervisor does not bypass Blaze's layout guard.
Busy work is retried; permanent failures wait for another edit. Compile errors
leave the old app running. Static initializers may reset the edited static. See
`.agents/reports/blaze-live-integration.md` for the measured limitations.
The broader app trial is in `.agents/reports/blaze-live-app-trial.md`: the Fast
GPU-surface backend currently aborts under the pinned engine, and unregistered
view fields or service data do not survive routed restarts. Live mode launches
the compiler executable directly, without Cargo dev's application bundle;
bundle-relative resources and installation/update metadata can therefore differ.
Plain `rake dev` remains Cargo-based and needs no Blaze installation. The older
`--compiler blaze` laboratory option remains available separately.

### Registering restart state

A restart restores **only registered checkpoint data**. Other view fields and
service state reset, even when restoration reports success. Rake cannot detect
fields omitted from a registered checkpoint (including counters like Kit's).
Capture every value you need to retain; recreate handles, tasks and services.

For a routed view, define owned state:

```rust
#[rake::data]
struct Draft { edits: u32 }

impl rake::reload::ReloadState for Draft {
    const SCHEMA: &'static str = "my-app.draft";
    const VERSION: u32 = 1;
}
```

Inside `RouteViews::mount`, decode it and register a provider before returning
the view (imports omitted; see `examples/routing/src/lib.rs` for the full mount):

```rust
let entry = page.entry();
let restored = entry.reload_state::<Draft>()
    .map_err(|error| MountError(error.to_string()))?;
let edits = Rc::new(Cell::new(restored.map_or(0, |state| state.edits)));
let captured = edits.clone();
entry.on_reload(move || Draft { edits: captured.get() }.checkpoint())
    .map_err(|error| MountError(error.to_string()))?;
// Store `edits` in the view; do not create a second independent counter.
```

Explicit provider/serialization errors still fail capture. Partial restart only
relaxes a missing provider; it does not bypass quiescence, size limits, schema
validation or a failed restore.

### Fork maintenance

The fork is prepared in `~/Documents/GitHub/blaze-rake-fork`; its `RAKE_FORK.md`
describes the verified upstream base (`v0.20261030.1`, `6560553`), the seven
engine fixes, eight separately preserved earlier lab fixes, matching toolchain
macro server and Apple Silicon release recipe. Its infrastructure workflows are
archived; only explicit fork release workflows remain active.

The first fork release is published. Its GitHub-built archive differs from the
local build, so the pin records the published archive's checksum. For future
releases, verify the actual public bytes before updating the pin. Local rebuilds
can have different checksums. Fork and Rake changes still need separate owner
approval before pushing or publishing; local maintenance does neither.

Allow roughly 2–4 hours per compatible upstream bump, and a day or more for
compiler/Cranelift/proc-macro ABI changes. The release branch now retains
upstream history: rebase it onto the next upstream tag using `rake/upstream-base`,
review every inherited fix, then rerun installation, six-trial text/helper reload
acceptance, and relevant compiler differential tests. The recovered root remains
only on the fork's local `snapshot-archive` branch.

### Fresh apps with Blaze

On Apple Silicon macOS:

```sh
rake new hello
cd hello
rake dev --live
```

The first live run downloads and verifies the pinned Blaze bundle. No global
Blaze install or manual feature flags are needed. The generated manifest exposes
`live = ["rake/live"]`, `devtools = ["rake/devtools"]` and
`gpui-fast = ["rake/gpui-fast"]`. Cargo includes their optional dependencies when
creating the lockfile, while `--live` enables only live runtime and devtools.
This also keeps the two GPUI backends on a compatible Taffy version. Plain
`rake dev` still uses Cargo and the default backend.

Older generated apps can add these aliases and run `cargo generate-lockfile`
once before `rake dev --live`. Keep that lockfile committed. Blaze's pinned
planner can request inactive optional dependencies absent from a Cargo lockfile;
this is an engine limitation, not a reason to enable every app feature at runtime.
A restart restores only registered checkpoints; other fields reset with a notice.
Use `rake dev --live --require-state` to block a restart that cannot restore state.
