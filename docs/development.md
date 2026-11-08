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

Each launch prints a private `/tmp/rake-dev-*` session directory. Its mode-0700
control directory isolates native instances and browser advertisements; checkpoints
and supervisor status use mode 0600. Inspect it with:

```sh
target/debug/rake dev status --session /tmp/rake-dev-EXAMPLE --json
RAKE_CONTROL_DIR=/tmp/rake-dev-EXAMPLE/control target/debug/rake instances --json
RAKE_CONTROL_DIR=/tmp/rake-dev-EXAMPLE/control target/debug/rake ctl reload status --json
```

Desktop dev launches the native app; `--preview` opens its browser preview.
Web dev opens the app directly. With `--cdp`, it opens the embedded
[Chrome DevTools](devtools.md) toggle instead. Node 22+ is required for Web
`--cdp`; frontend assets download when the panel opens. `--no-open` prints the
private launch URL without opening a browser. Build and reload status is also
available through `rake dev status --session PATH`.

Use `rake dev --cdp` to start the Chrome DevTools adapter with the app, or
`rake inspect --session PATH` / `rake cdp --session PATH` to attach to an existing
session. `--instance UUID` selects a runtime. The legacy HTML and GPUI inspectors
are disabled and are not bundled.

`--json` emits supervisor events as JSON lines, and compiler and app diagnostics go to stderr. `status.json` stays readable when the app is stopped or crashed.

Ctrl-C or SIGTERM stops owned app, build, compiler, server and backend process groups and removes their discovery files. Temporary session artifacts and a failed checkpoint remain for recovery. Remove the session directory when you're done, because it may contain unsaved development data and large build artifacts.

## Shared runtime boundary

Routed and standalone root apps opt in with `ControlConfig::FromEnvironment`. Native dev sets `RAKE_DEV_CONTROL=1` and a private control directory, and Web uses the authenticated same-origin development bridge. Ordinary builds enable neither.

`App::root` keeps its `Services` mount signature. `App::root_with` receives a weak `RuntimeContext` with the managed command, task, quit and update-restart boundary.

Standalone roots have no router navigation or checkpointed workspace, so their reload is a fresh root replacement. The supervisor first ensures the old host reaches the managed runtime boundary. If that can't be coordinated, the old app stays usable, and `--restart-without-state` explicitly allows a restart without restored state.

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

This is currently **Apple Silicon macOS only**. The fork repository/release is
not public yet. Until the owner publishes the pinned bytes, use the locally
prepared archive:

```sh
BLAZE_ARCHIVE=/absolute/path/to/blaze-v0.20261030.1-rake.1-aarch64-apple-darwin.tar.gz rake dev --live --bin rake-view
```

`BLAZE_ARCHIVE` supplies bytes, not a checksum bypass. `RAKE_BLAZE=/absolute/path/to/blaze`
is an explicit developer override; it requires the patched
`rust-analyzer-proc-macro-srv` beside that executable and bypasses the release pin.
Doctor reports the pinned installation's platform, version, checksum and server
readiness without installing anything.

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
apps must register the state they want restored. Compile errors leave the old
app running. Static initializers may reset the edited static. See
`.agents/reports/blaze-live-integration.md` for the measured limitations.
Plain `rake dev` remains Cargo-based and needs no Blaze installation. The older
`--compiler blaze` laboratory option remains available separately.

### Local fork maintenance and first publication

The fork is prepared in `~/Documents/GitHub/blaze-rake-fork`; its `RAKE_FORK.md`
describes the verified upstream base (`v0.20261030.1`, `6560553`), the seven
engine fixes, eight separately preserved earlier lab fixes, matching toolchain
macro server and Apple Silicon release recipe. Its infrastructure workflows are
archived; only explicit fork release workflows remain active.

Owner-only public steps remain: create `1bye/blaze` as a GitHub fork of
`indexable-inc/blaze`, push the reviewed fork branch,
cut the first `v0.20261030.1-rake.1` prerelease, then confirm/update the pin URL
and SHA-256 against the exact published archive. Publishing the already-tested
local archive preserves its checksum; a CI rebuild may produce different bytes
and needs a new pin. Rake's new branch also needs a separate push/review approval.
No public step is performed by local preparation.

Allow roughly 2–4 hours per compatible upstream bump, and a day or more for
compiler/Cranelift/proc-macro ABI changes. The release branch now retains
upstream history: rebase it onto the next upstream tag using `rake/upstream-base`,
review every inherited fix, then rerun installation, six-trial text/helper reload
acceptance, and relevant compiler differential tests. The recovered root remains
only on the fork's local `snapshot-archive` branch.
