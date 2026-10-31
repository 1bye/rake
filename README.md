# Rake

Rake is an application framework for native GPUI and browser WebAssembly. This
repository implements the **portable foundation, routing, managed work, runtime control, development supervision, graphical devtools, application packaging, element inspection, typed application commands, guarded quit and native multi-window routing** of the
[API design](docs/api-design.md). It is an unpublished prototype.

The foundation includes:

- One `rake` facade, a small prelude, and advanced GPUI access through `rake::gpui`.
- A shared application factory with native startup and a Wasm start export generated
  by `#[rake::app]`. A single root view mounts on the host UI event loop.
- Shared HTTP configuration, native HTTP and browser Fetch, JSON bodies and responses,
  explicit cancellation, total deadlines and incremental body consumption.
- Bounded structured logs, function spans and request lifecycle records. The default
  runner installs an INFO-level subscriber and terminal/browser-console output.
  Embedding hosts can attach `Collector::layer()` to their own subscriber instead.
- Facade-aware `#[rake::data]` and `#[rake::instrument]`, including renamed dependencies.
- Typed destinations and URL codecs, independent application tabs, retained per-tab
  histories, eager operation handles, asynchronous access and disposal guards,
  revision-checked decisions, and browser path/hash history adapters.
- Native windows with independent tabs, selections and histories; window-scoped
  navigation, guarded window closing and compatible multi-window reload restoration.
- Local tab/application task scopes, fresh entry activity runs, Foreground/Live
  policies, visibility-only activity, checked typed publication and cancellation.
- Shared runtime commands/queries/events, typed application command derives and registration,
  retained operation results and decisions, retry-safe requests, the `rake` terminal
  client, private Unix IPC and an authenticated browser development bridge.
  Schema-enabled data types generate accepted argument and serialized result schemas;
  GUI, devtools and CLI calls share retry-safe command operations.
- Guarded application quit across retained histories, save/discard/cancel decisions,
  explicit running-job policy and shared GUI/CLI/devtools dispatch.
- Native/Web build and dev commands, debounced build watching, coordinated reload,
  versioned workspace restoration and an external native reloader boundary.
- A supervisor-owned browser inspector for native and Web runtimes: workspace,
  routes, tasks, operations, guard decisions, console, HTTP requests and span trees.
  Every action uses the same runtime dispatcher as the CLI.
- Bounded element inspection: presented boundary tree, real layout, authored styles,
  source sites and semantic IDs through shared queries and the Elements panel.
- Native application/resource bundles, macOS signing and dylib relocation, Web
  deployment archives with path/hash prefixes, manifests and archive checksums.

See the [routing example](examples/routing/src/lib.rs) and
[routing contract](docs/routing.md). Both examples depend only on `rake`, and their
shared factories build for native and browser targets. The
[renamed consumer](tests/renamed-facade/src/lib.rs) checks macro-generated paths.
See [guarded quit](docs/guarded-quit.md) for shutdown decisions and host limits.
See [managed work](docs/managed-work.md) for ownership and host scheduling, and
[runtime control](docs/runtime-control.md) for CLI setup, shared handlers and bounds.
See [development and reload](docs/development.md) for `rake dev`, builds and checkpoint integration.
See [graphical devtools](docs/devtools.md) for launch commands, panels and capture limits.
See [element inspection](docs/element-inspection.md) for instrumentation and coverage.
The opt-in [`rake::view!` experiment](docs/experimental-view.md) compiles HTML-like
markup directly into GPUI elements, with utility styles, events, conditions and
loops. Run `cargo run -p rake-view` for the native example, or build it for Web
with `python3 scripts/build-web.py --example view`.
See [packaging and distribution](docs/packaging.md) for bundle metadata, assets and deployment.

The [Fieldnotes reference app](docs/notes.md) provides a complete notes workspace
with search, pinned notes, an editor, autosave and crash recovery. Run
`cargo run -p rake-notes`. Its [typed service composition](docs/services.md) keeps
GUI edits, CLI commands and lifecycle guards on the same notebook instance.

## Workspace

`rake` is the application facade. It owns routing, task lifetimes, runtime control
and GPUI host integration. Existing `rake::http`, `rake::diagnostics`, routing,
inspection, theme and overlay imports retain their types through reexports.

- `rake-diagnostics` owns bounded records, tracing context, snapshots and native/Web
  output. It builds independently of GPUI, HTTP and procedural macros.
- `rake-http` owns HTTP, cancellation and executor-portable timers. Native requests
  use the caller's executor; browser requests use Fetch. Its optional `task-errors`
  integration preserves HTTP-to-task error conversion when used through `rake`.
- `rake-protocol` owns route codecs, IDs, snapshots, checkpoint data and versioned
  control messages. It has no UI, HTTP or application-runtime dependency.
- `rake-client` owns Unix discovery and blocking, deadline-bounded local/Web control
  exchanges on native targets, with inspectable transport and runtime errors.
  Unix connection/write/read work shares one deadline. Web exchanges reject redirects
  and cap response bodies at 1 MiB; endpoint debug output redacts credentials. It depends on
  `rake-protocol` and `rake-http`; it does not start an application runtime.
- `rake-ui` owns GPUI view helpers, themes and overlays. The facade enables it with
  `experimental-view`. Its `test-support` feature enables native frame tests.
- `rake-macros` generates facade-aware application, route and command syntax. Its
  `view` feature compiles the experimental markup/theme/style compiler and is
  activated by the facade's `experimental-view` feature.
- `rake-cli` provides terminal control and CDP through `rake-client`,
  `rake-protocol` and `rake-diagnostics`. Only its test fixtures depend on `rake`.

Runtime adapters depend on these smaller crates. Router and task state stay together
because their entry, window, cancellation and reload invariants share one local owner.
Application code can continue depending only on `rake`.

Inspector polling filters diagnostics by cursor/kind and tasks by owner before
cloning. Unchanged event polls check the revision without constructing a snapshot.
Targeted control queries copy only their requested data. Window snapshots
encode only that window's histories. Retained inspection captures share immutable
storage internally; external snapshots remain independent owned values. Style-only
redraws keep element identities, while changed boundaries invalidate the generation.

Run the focused release measurements with:

```sh
cargo bench -p rake-diagnostics --bench diagnostics_snapshot --locked
cargo bench -p rake --no-default-features --bench control_queries --locked
cargo test -p rake --no-default-features --features devtools --release --lib publish_benchmark --locked -- --ignored --nocapture
```

On the local synthetic fixture (128 tabs, 640 history entries and 256 tasks), median
query times over five batches changed from 125.0 to 0.4 µs for windows, 137.5 to
18.2 µs for operations, and 119.2 to 0.9 µs for an empty owner-filtered task result.
Publishing 256 unchanged inspection boundaries with 512-byte style payloads changed
from 203.2 to 51.1 µs. A subsequent unchanged-event polling fast path reduced that
query from 279.6 to 0.022 µs on the same fixture, excluding serialization and I/O.
These measure CPU work, not end-to-end GPU/frame performance.

## Run

The project pins Rust 1.95.0 and GPUI CE source revision
`99109e3a77ddadbcb46cba8567e779a651600608` (GPUI 0.2.2 and platform 0.1.0).
The source pin includes font assets missing from the published Web crate. GPUI CE
is the selected community GPUI distribution, rather than a moving Zed checkout.
Cargo.lock pins the complete resolution. Initial native validation targets macOS;
other desktop backends need their platform development libraries and validation.

```sh
cargo run -p rake-foundation
cargo run -p rake-routing
```

The example's **Load JSON** button requests `http://127.0.0.1:8080/message.json`.
Start the fixture server for either native or browser use:

```sh
python3 -m http.server 8080 --bind 127.0.0.1 --directory examples/foundation/web
```

For the browser, install `wasm-bindgen-cli` at the exact `wasm-bindgen` version in
Cargo.lock (the build script prints the command when needed), then:

```sh
python3 scripts/build-web.py
python3 -m http.server 8080 --bind 127.0.0.1 --directory dist/foundation
```

Open `http://127.0.0.1:8080`. The script packages the shared Wasm application,
generated bindings, loader, host document and JSON fixture. GPUI owns its canvas
and GPU resources. This initial adapter uses a single-threaded browser host.

For the routing example, the development server serves the app shell for deep links:

```sh
python3 scripts/build-web.py --example routing
python3 scripts/serve-web.py --example routing --port 8081
```

Open `http://127.0.0.1:8081/documents/A`. Open A twice, edit one instance,
switch tabs, and use both per-tab and browser Back. Closing a dirty tab offers
Save, Discard and Cancel through the same operation decision interface.
Its refresh and preview counters show activity restarts. Select Live to keep refresh
running in a hidden tab; preview always follows visibility. Downloads belong to a
tab, while exports survive closing their originating tab.
To use fragment URLs without server rewrites:

```sh
python3 scripts/build-web.py --example routing --hash
python3 scripts/serve-web.py --example routing-hash --port 8082
```

Open `http://127.0.0.1:8082/#/documents/A`. Production path deployments must
provide their own app-shell rewrites and serve the generated assets at `/pkg/`.

## Verify

```sh
cargo fmt --all --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy -p rake-client -p rake-http -p rake-protocol -p rake-diagnostics --lib --locked -- -D warnings -D clippy::missing_errors_doc -D clippy::must_use_candidate -D clippy::return_self_not_must_use
cargo test --workspace --locked
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
cargo check -p rake --no-default-features --locked
cargo check -p rake-diagnostics -p rake-protocol -p rake-http --lib --target wasm32-unknown-unknown --locked
cargo check -p rake-macros --no-default-features --locked
cargo test -p rake-client -p rake-http -p rake-protocol --locked
cargo test -p rake-ui --features test-support --lib --locked
cargo test -p rake --no-default-features --test managed_work --test routing --test runtime_control --test reload --test guarded_quit --locked
cargo build -p rake-cli --locked
python3 scripts/test-dev-bridge.py
python3 scripts/test-supervisor.py
python3 scripts/test-bundle.py
python3 scripts/test-inspector.py
node scripts/test-inspector.mjs
node scripts/test-inspector-ui.mjs
cargo check -p rake-foundation -p rake-routing -p renamed-facade --lib --target wasm32-unknown-unknown --locked
cargo clippy -p rake -p rake-foundation -p rake-routing -p renamed-facade --lib --target wasm32-unknown-unknown --locked -- -D warnings
cargo check -p rake-routing --lib --features hash-routing --target wasm32-unknown-unknown --locked
python3 scripts/check-spacing.py
```

Native HTTP tests use actual loopback TCP and the real transport on a non-Tokio
executor. No external API or credentials are required. Target compilation does
not establish browser/GPU execution; the browser example needs a real browser.

Both examples were smoke-tested on macOS and in Helium's Chromium browser.
The routing checks exercised independent drafts, retained history, duplicate
destinations, dirty close decisions, and browser Back with path and hash URLs.
The foundation's **Load JSON** displayed the fixture response on both targets.
Browser startup retains GPUI after the Wasm start callback returns. Other native
platforms and browsers have not been runtime-tested.

Rust rules and the `rust-development` skill are installed from the local workflow
repository under `.agents/`. The skill is available to subsequent agent turns.
The Dylint spacing library is pinned to workflow commit
`f62c7d7dcdf3ffd9b77de95a523544a0c688bb9b`. It uses a separate development-only
nightly and does not change the application's stable toolchain:

```sh
rustup toolchain install nightly-2026-08-20 --profile minimal --component rustc-dev,llvm-tools-preview,rust-src,rustfmt
cargo +nightly-2026-08-20 install cargo-dylint dylint-link --version 6.2.0 --locked
python3 scripts/check-spacing.py --fix
cargo fmt --all
python3 scripts/check-spacing.py
```

The checker rejects spacing warnings as well as compiler errors. The fixer applies
the library's whitespace suggestions and then reruns the check.

## Current boundary

Packaging, signing and publication are described in [distribution](docs/distribution.md).
Durable sessions remain separate. Native code
replacement awaits the external hot reloader; native dev currently uses an explicitly
reported checkpointed restart. Runtime control is opt-in, native IPC currently requires Unix, and state watches coalesce with
explicit revision gaps. Native routed applications support multiple windows; Web
keeps one host and reports `NewWindow` as unsupported. Host visibility is measured in Web
and on macOS; other native backends report Unknown. Development reload checkpoints
restore compatible workspaces and opt-in entry data; durable sessions are separate.
`App::root` and `App::root_with` support managed applications without navigation.
See [application shapes](docs/application.md) for root, background and headless hosts.

HTTP request diagnostics cover Rake clients and the native GPUI adapter; browser
GPUI-internal requests currently use GPUI's own Fetch adapter. Third-party clients
are not captured. Metadata excludes authorization/cookie headers, request/response
bodies, URL credentials, query strings and fragments. Body completion is distinct
from headers; dropping a body records abandonment. Browser network failures do
not claim to identify CORS errors. Development control is disabled by default; enable it explicitly as described in
[runtime control](docs/runtime-control.md).

`send` preserves non-2xx statuses; `send_json` requires 2xx and then decodes JSON.
Relative endpoints require an explicit HTTP(S) base on both targets. Cancellation
and timeouts stop waiting and do not undo server effects. Async HTTP yields to the
host executor; it does not make CPU-heavy application work run on worker threads.
