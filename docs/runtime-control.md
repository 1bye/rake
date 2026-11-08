# Runtime control and terminal parity

Project dev/build commands and the shared reload boundary are described in
[development and reload](development.md).

`rake::control` owns the versioned command, query and event protocol. The local `RuntimeClient`, the `rake` terminal binary, Unix transport and Web development broker all submit the same owned messages.

Routing messages call the existing `Navigator` and `TabHandle` operations, including their guards and revision checks. Application handlers run on the host event loop with local futures, and no UI borrow or `Send` requirement crosses a transport boundary.

## Enable development control

The default is `ControlConfig::Disabled` in both debug and release builds. Routed
applications can explicitly opt into the runner's development environment:

```rust,no_run
use rake::control::ControlConfig;
use rake::prelude::*;
// In the application's existing factory:
// App::new("example.notes").routes(Views).control(ControlConfig::FromEnvironment)
```

`App::root(...).control(ControlConfig::FromEnvironment)` and `App::root_with(...).control(ControlConfig::FromEnvironment)` opt a standalone root into the same native environment flag or authenticated Web metadata. `App::root` keeps its `Services` closure signature, and `App::root_with` also receives a weak `RuntimeContext` for managed tasks, commands, guarded quit and update restart.

A root has no navigation, tabs or retained workspace, but its `RuntimeClient<NoRoute>` still exposes commands, managed work, guarded quit, inspection and updater actions. Unsupported navigation requests return `ControlError::Unsupported`, so clients must use the advertised capabilities. The HTML inspector omits queries for absent workspace state.

Native root disposal removes its discovery files and socket, and Web disposal aborts its authenticated bridge and outstanding I/O. `App::background(...)` gives a native-only host the same managed context without opening an initial window.

Standalone roots can register commands, quit guards and managed work in the same builder as routed apps. `App::routes` is still required for navigation, retained tabs and entries, and checkpointed restoration. Headless hosts can construct a `Router<NoRoute>`, register handlers and drive it without a GUI.

All hosts are disabled by default, in debug and release builds.

Build and run the terminal client locally:

```sh
cargo build -p rake-app-cli
RAKE_DEV_CONTROL=1 cargo run -p rake-routing
# In another terminal:
target/debug/rake instances --json
target/debug/rake ctl --instance RUNTIME_UUID routes list --json
target/debug/rake ctl --instance RUNTIME_UUID router open document \
  --param id=A --target new-tab --mode live --json
```

In subsequent examples, `rake` means `target/debug/rake`; installing with
`cargo install --path crates/rake-cli` also makes that command available on PATH.

Use `--preserve-selection` independently of `--mode`. NewTab always means an
internal application tab. NewWindow creates a physical native host with its own
selection and histories. Web and custom hosts without an adapter report an
unsupported capability and omit it from advertised targets.

```sh
rake ctl windows list --json
rake ctl windows get window-4 --json
rake ctl router open document --param id=A --target new-window --json
rake ctl router open document --param id=B --target current-tab --window window-4 --json
rake ctl router reveal document --param id=A --window window-4 --scope window --json
rake ctl windows close window-4 --json
```

- `Open.window` and `Reveal.window` optionally name a host. Omitting it binds the initial host.
- `Reveal.scope` defaults to Application, and Window restricts reuse.
- Exact-tab commands infer their owning window.
- `CloseWindow` shares GUI close guards, operation status and decision responses.
- `Query::Windows` lists hosts, and `Query::Window` returns a host-scoped routing snapshot.

All windows share one runtime and transport. The global snapshot adds per-window selections, visibility, closing operations, tab membership and each decision's presenting host. After the initial host closes, name a remaining host explicitly. App-level NewWindow can reopen a host even after all windows have closed.

Unix hosts create an owned mode-0700 directory at `/tmp/rake-UID` (or `RAKE_CONTROL_DIR`), a mode-0600 Unix socket and a private discovery JSON file. The host owns and removes the socket and advertisement.

A stale file can remain after a crash, so the CLI probes advertisements before selecting a live runtime. When several live runtimes match, `--instance` must select a RuntimeId; a unique application ID also works. Native IPC supports Unix only, and Windows local transport isn't implemented.

For the browser:

```sh
python3 scripts/build-web.py --example routing
rake dev -p rake-routing --target web --port 8081
# Open the private launch URL printed by the server, then use rake instances / rake ctl.
```

`--control` prints a private launch URL whose fragment authenticates this browser session.

1. A parser-blocking bootstrap script removes the fragment before Rake reads the route, and stores the credential in session storage for this origin and tab. Authenticating an already-open page reloads it.
2. Reloads reconnect from session storage. Open the launch URL again after restarting the broker, which rotates its token.
3. The running Wasm instance starts authenticated HTTP long polling, registers its RuntimeId and capabilities, receives the same protocol requests and sends the same replies.

The broker only forwards messages, and the browser dispatcher validates and executes them. It grants no native filesystem, process or window privileges.

**Security:** the server accepts loopback traffic, validates Host and Origin, and requires a random session token. The token stays in the private launch URL, owner-only advertisements and the authenticated browser session, and `rake instances` omits it. Publicly served HTML and JavaScript contain no credential, the packaged files are unchanged, and static production builds contain no connection metadata and need no broker. The injected bootstrap supports HTML with an explicit or implicit head.

`rake ctl --timeout SECONDS` applies to the runtime identity probe as well as
request exchange and operation observation. Hidden browser tabs can respond more
slowly than native sockets; selecting an explicit runtime must not impose a
separate fixed two-second discovery deadline.

## Commands, queries and outcomes

`target/debug/rake --help` lists the terminal mappings. Every protocol variant can also use `rake ctl request JSON_MESSAGE`, and Rust's shared Serde types validate both paths. `ctl schema`, `routes list` and `commands list` discover protocol tags, route parameters and command schemas.

Unknown tags and wire fields fail decoding. JSON uses numeric tab, entry and operation IDs, and the friendly CLI accepts their printed `tab-N`, `entry-N` and `op-N` forms.

**Commands:** open and reveal, exact-tab navigate, replace, select and close, Back and Forward, policy changes, task and operation cancellation, decision responses, [guarded quit](guarded-quit.md) and registered application commands.

**Inspection:** `inspect tree` and `inspect element ELEMENT_ID` expose [element inspection](element-inspection.md) snapshots, coverage and exact capture lookups.

**Queries:** runtime capabilities, application state, routes, tabs and histories, tasks and owners, operations and results, pending decisions and bounded diagnostics.

**Diagnostics:** logs, request records and span exports read the same collector as the [graphical inspector](devtools.md). Span IDs, parent IDs, inherited causal fields and elapsed wall time support bounded trace trees, and missing observations stay explicit.

A submission returns an operation ID before commit. The terminal waits for a terminal state or pending decision by default, and `--no-wait` returns immediately. `ctl quit` always returns on submission, because native exit can remove the transport. `ctl quit status` reports its phase and pending jobs while the runtime is alive.

An operation query returns queued, running, awaiting-decision, committed, cancelled, blocked or failed, plus committed result IDs or the application result. It works equally for GUI-submitted operations.

Completed states and results share `RouterConfig::completed_operations` retention, the most recent 256 by default, and all pending states are retained. Eviction returns `OperationExpired`, and an unissued ID returns `UnknownOperation`.

```sh
rake ctl --instance RUNTIME_UUID tabs close tab-4 --no-wait --json
rake ctl --instance RUNTIME_UUID operations get op-3 --json
rake ctl --instance RUNTIME_UUID decisions list --json
rake ctl --instance RUNTIME_UUID decisions respond decision-7 --choice save
rake ctl --instance RUNTIME_UUID operations watch op-3 --json
rake ctl --instance RUNTIME_UUID operations cancel op-3
rake ctl --instance RUNTIME_UUID tasks list --owner tab-4 --json
rake ctl --instance RUNTIME_UUID tasks cancel task-3
```

Disconnection, a watch timeout or dropping an observation never approves a
pending decision or cancels an operation. A stale choice returns `Obsolete`.
Cancellation takes priority over simultaneously ready work before commit. After
commit cancellation returns false. Cancellation cannot undo external side effects
already performed by an application handler.

Application and router mutations need a 1–128 byte request key. The CLI generates one and includes it in its result or in an uncertain-response error. Supply `--request-key KEY` to retry explicitly.

- An identical request returns its original reply.
- Reusing a key for another payload returns `KeyConflict`.
- Keys and failures stay reserved for the whole runtime lifetime.
- `RouterConfig::control_requests` limits admission to 1024 distinct keys by default. Exhaustion returns `RequestCapacity` rather than forgetting a key and duplicating an open. This bound is separate from result retention.
- A new runtime rejects old runtime envelopes, and keys don't span a process restart or full page reload.

Transient development commands work differently. `highlight_element` and `inspect_mode` use bounded leases, return acknowledgements, and keep no operation or key entry. `navigate_element` takes an exact ID and generation plus `action: focus|scroll_into_view`, returns an acknowledgement, and is never replayed automatically after uncertain delivery. Unsupported handles and stale captures return typed inspection errors. See [highlight and picking](element-inspection.md#development-highlight-and-picking).

## Explicit typed application commands

`#[derive(rake::Command)]` implements `ApplicationCommand` from explicit `#[command(id = "stable.key", output = ResultType, capability = Headless)]` metadata. Pair it with `#[rake::data(schema)]` to generate Serde codecs and JSON Schema through the facade.

Existing `#[rake::data]` types keep their Serde-only behavior. Result and domain types can also use `data(schema)`, including nested structs, enums, newtypes and generic containers. Neither Serde nor Schemars needs to be a direct dependency, even if `rake` is renamed.

```rust
use rake::prelude::*;
use rake::control::{ApplicationCommand, Reply};
use rake::router::NoRoute;

#[rake::data(schema)]
#[derive(rake::Command)]
#[command(id = "document.describe", output = Description, capability = Headless)]
#[serde(deny_unknown_fields)]
struct Describe { document: String }

#[rake::data(schema)]
struct Description { title: String }

let router = Router::<NoRoute>::new(RouterConfig::default());
router.client().commands().register::<Describe, _, _>(|arguments, context| async move {
    // Capture services/models explicitly. context.app.tasks() is application-owned;
    // context.cancellation is the operation's cooperative cancellation signal.
    Ok(Description { title: format!("Document {}", arguments.document) })
}).unwrap();
let reply = router.navigator().dispatch(Describe { document: "A".into() }, "request-1".into()).unwrap();
assert!(matches!(reply, Reply::Submitted { .. }));
// The host drives router.run(); observing a handle does not execute the command.
```

A GUI factory can use `App::commands(registry)` or fallible `App::command::<Describe, _, _>(handler)` before startup. The latter returns `Result<App<_>, CommandError>`, so duplicate keys never silently replace handlers. Commands work in routed apps, managed standalone roots and headless `Router<NoRoute>` hosts.

- The registry accepts at most 256 commands, and registration freezes each descriptor.
- The derive needs a concrete struct and a 1–128 byte ID without whitespace. Unknown or duplicate options fail compilation.
- Named, tuple, newtype and unit structs are supported.
- `output` defaults to `()`. `capability` defaults to `RequiresWindow`, so headless execution needs an explicit declaration.

Generated descriptors contain self-contained Draft 2020-12 schemas with local `$defs` and `$ref` references. Arguments describe **deserialization**, and results describe **serialization**, so Serde renames, defaults, optional and skipped fields follow the direction used at that boundary.

- Result types must implement Serde serialization and deserialization and JSON Schema.
- For custom codecs, use Schemars annotations through `data(schema)` or `rake::schemars`, or implement `ApplicationCommand` manually with honest schemas.
- Schema constraints are descriptive, not a runtime validator. Validate domain rules in the handler or a validating `Deserialize`.
- Argument codecs must round-trip for typed GUI dispatch.
- Unknown fields follow Serde policy, so add `deny_unknown_fields` when they should fail.

`commands list` returns the registered argument/result schemas and capability.
The Console panel uses those same schemas for editable JSON templates, including
local references and tuple/unit inputs; templates are not schema validators.
The terminal equivalent of the GUI call is:

```sh
rake ctl commands list --json
rake ctl commands run document.describe --args '{"document":"A"}' --request-key request-1 --json
```

`PageContext::dispatch`, `Navigator::dispatch`, `RuntimeClient::dispatch` and the CLI all lower to the same `Command::Run` request, retry key and operation registry. The derive supplies the command ID without rebuilding schemas.

Typed arguments decode before admission, and handlers start only when the driver polls them. `RequiresWindow` fails before invocation when no window is mounted. `Headless` describes requirements, and doesn't launch a headless process. The Web bridge grants no extra native capabilities.

Operation IDs, status, cancellation, failure, result retention and retry rules are identical for GUI and CLI calls. Results are kept as JSON in `OperationRecord.result`, so decode with the declared output type where its Serde contract permits.

Handlers don't hold a router scheduling lane, so a handler can await navigation without deadlocking, and they own their model transaction rules. A committed command means the handler returned successfully. It doesn't assert presentation or data readiness, or roll back external effects when cancellation wins. No view or borrowed GPUI context crosses await.

## Watches and transport bounds

`RuntimeClient::watch()` captures an owned snapshot and revision on the local event loop. `next()` checks that revision before waiting, so changes before the first poll are seen. `Query::Events` and `rake ctl events --follow` expose the same runtime, epoch and revision cursor.

State is coalesced and intermediate mutations aren't replayed. A skipped revision, stale epoch or other runtime returns `Gap` plus a fresh snapshot, and an unchanged poll contains no snapshot. Consumers replace their state on Snapshot, Changed or Gap. Tracing loss never defines authoritative routing or operation state.

`logs follow`, `requests list`, `traces export` and raw Diagnostics queries expose
collector sequence numbers and cumulative dropped/output-dropped counts. They are
best-effort diagnostics and have separate retention from state events.

| Limit | Value |
| --- | --- |
| Transport frame | 1 MiB. Oversized snapshots fail framing, and there's no pagination yet |
| Native IPC | 32 active connections, one message each, five-second I/O deadline |
| Broker | 32 browser instances, 128 queued requests (32 per instance), 64 simultaneous HTTP handlers |
| Client observation | Times out after 10 seconds without cancelling the queued command |
| Tickets | Expire after 60 seconds |
| Browser leases | Expire after 30 seconds without polling, requiring reconnect |

Undelivered or reply-lost requests may be redelivered with the same request key. A hidden browser may throttle or freeze polling, and reconnecting keeps the same RuntimeId while that app instance survives.

The [development supervisor](development.md), [build/packaging commands](packaging.md), reload boundary and
graphical inspector use this control surface. Project packaging runs without a live
runtime and enables no development transport in its Web output. Application commands
use the same registry, including generated metadata.

## Verification

```sh
cargo test -p rake-app --no-default-features --test runtime_control --test application_commands
cargo test -p rake-app-cli
cargo build -p rake-app-cli
python3 scripts/test-dev-bridge.py
```

Native tests use real Unix sockets and terminal child processes. Broker checks use real loopback HTTP and the compiled CLI. A Wasm browser smoke check covered CLI-driven tabs, retries, Save decisions and task cleanup.
