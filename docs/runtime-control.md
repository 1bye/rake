# Runtime control and terminal parity

Project dev/build commands and the shared reload boundary are described in
[development and reload](development.md).

`rake::control` owns the versioned command, query and event protocol. The local
`RuntimeClient`, the `rake` terminal binary, Unix transport and Web development
broker submit the same owned messages. Routing messages call the existing
`Navigator`/`TabHandle` operations, including their guards and revision checks.
Application handlers run on the host event loop with local futures. No UI borrow
or `Send` requirement crosses a transport boundary.

## Enable development control

The default is `ControlConfig::Disabled` in both debug and release builds. Routed
applications can explicitly opt into the runner's development environment:

```rust,no_run
use rake::control::ControlConfig;
use rake::prelude::*;
// In the application's existing factory:
// App::new("example.notes").routes(Views).control(ControlConfig::FromEnvironment)
```

The current runner adapter requires `App::routes`; configuring control or commands
on a root-only `App::root` returns `ControlRequiresRouter` before host startup.
Headless hosts can construct a `Router<NoRoute>`, register handlers and drive it
without a GUI.

Build and run the terminal client locally:

```sh
cargo build -p rake-cli
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

`Open.window` and `Reveal.window` optionally name a concrete host; omission binds
the initial host. `Reveal.scope` defaults to Application; Window restricts reuse.
Exact-tab commands infer their owning window. `CloseWindow` shares GUI close guards,
operation status and decision responses. `Query::Windows` lists hosts and
`Query::Window` returns a host-scoped routing snapshot. All windows share one runtime
and transport. The global snapshot adds per-window selections/visibility/closing
operations, tab membership and the presenting host of each decision. After the initial host is
closed, explicitly name a remaining host; app-level NewWindow can reopen a host
even after all windows have closed.

Unix hosts create an owned mode-0700 directory at `/tmp/rake-UID` (or
`RAKE_CONTROL_DIR`), a mode-0600 Unix socket and a private discovery JSON file.
The host owns and removes the socket and advertisement. After a crash a stale file
can remain; the CLI probes advertisements before selecting a live runtime. When
more than one live runtime matches, `--instance` must select its RuntimeId. A
unique application ID is also accepted. Native IPC currently supports Unix;
Windows local transport is not implemented.

For the browser:

```sh
python3 scripts/build-web.py --example routing
python3 scripts/serve-web.py --example routing --port 8081 --control
# Open the private launch URL printed by the server, then use rake instances / rake ctl.
```

`--control` prints a private launch URL whose fragment authenticates this browser
session. A parser-blocking bootstrap script removes the fragment before Rake reads
the route and stores the credential in session storage for this browser origin/tab.
Authenticating an already-open page reloads that page.
The script creates the connection metadata in memory; publicly served HTML and
JavaScript contain no credential. Reloads reconnect from session storage. Open the
launch URL again after restarting the broker, which rotates its token. The packaged
files remain unchanged. The running Wasm instance initiates authenticated
HTTP long polling, registers its own RuntimeId and capabilities, receives the
same protocol requests and sends the same replies. The broker only forwards
messages; the browser dispatcher validates and executes them. It grants no
native filesystem, process or window privileges. Static production builds contain
no connection metadata and require no broker. The server accepts loopback traffic,
validates Host/Origin, requires a random session token and keeps the token in the private launch URL, owner-only advertisements and
authenticated browser session. `rake instances` omits tokens.
The injected bootstrap supports HTML with either an explicit or implicit head.

`rake ctl --timeout SECONDS` applies to the runtime identity probe as well as
request exchange and operation observation. Hidden browser tabs can respond more
slowly than native sockets; selecting an explicit runtime must not impose a
separate fixed two-second discovery deadline.

## Commands, queries and outcomes

`target/debug/rake --help` lists the supported terminal mappings. Every protocol
variant can also use `rake ctl request JSON_MESSAGE`; Rust's shared Serde types
validate both that path and the friendly argument mappings. `ctl schema`,
`routes list` and `commands list` discover protocol tags, route parameters and
application command argument/result schemas. Unknown tags and unknown wire fields
fail decoding. The current JSON encoding uses numeric tab/entry/operation IDs;
the friendly CLI accepts their printed `tab-N`, `entry-N`, `op-N` forms.

The initial surface includes open/reveal, exact-tab navigate/replace/select/close,
Back/Forward, policy changes, task cancellation, operation cancellation, decision
responses, [guarded application quit](guarded-quit.md) and registered application commands. `inspect tree` and
`inspect element ELEMENT_ID` expose the [element inspection](element-inspection.md)
snapshots, coverage and exact capture lookups. Queries expose runtime capabilities,
application state, routes, tabs/histories, tasks/owners, operations/results, pending
decisions and bounded diagnostics. Logs, request records and span open/close exports
read the same collector as the [graphical inspector](devtools.md). Span IDs,
parent IDs, inherited causal fields and elapsed wall time support bounded trace
trees; missing observations remain explicit.

A submission returns an operation ID before commit. The terminal waits until a
terminal state or a pending decision by default. `--no-wait` returns immediately.
`ctl quit` always returns submission immediately because successful native exit
can remove the transport before a later operation query. `ctl quit status` reports
its phase and pending jobs while the runtime remains alive.
An operation query returns queued/running/awaiting-decision/committed/cancelled/
blocked/failed and the committed result IDs or application result. Completed
states and results share `RouterConfig::completed_operations` retention: the most
recent 256 by default. All pending states remain retained. Eviction returns
`OperationExpired`; an unissued ID returns `UnknownOperation`. Operation queries
work equally for GUI-submitted operations.

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

Every mutation requires a 1–128 byte request key. The CLI generates one and includes
it in its submission/result output, or in an uncertain-response error. Supply
`--request-key KEY` to retry explicitly. An identical request returns its original
reply; reusing its key for another payload returns `KeyConflict`. Keys and failures
remain reserved for the entire runtime lifetime. `RouterConfig::control_requests`
limits admission to 1024 distinct keys by default; exhaustion returns
`RequestCapacity` rather than forgetting a key and duplicating an open. This bound
is separate from completed-operation result retention. A new runtime rejects old
runtime envelopes. Retry keys do not span a process restart or full page reload.

## Explicit typed application commands

`#[derive(rake::Command)]` implements `ApplicationCommand` from explicit
`#[command(id = "stable.key", output = ResultType, capability = Headless)]`
metadata. Pair it with `#[rake::data(schema)]` to generate Serde codecs and JSON
Schema through the facade. Existing `#[rake::data]` types keep their original
Serde-only behavior. Result/domain types can also use `data(schema)`, including
nested structs, enums, newtypes and generic containers. Neither Serde nor Schemars
needs to be a direct application dependency, including when `rake` is renamed.

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

A GUI factory can use `App::commands(registry)` or fallible
`App::command::<Describe, _, _>(handler)` before startup. The latter returns
`Result<App<_>, CommandError>` so duplicate keys never silently replace handlers.
Application commands currently require the routed runner (`App::routes`). A
headless `Router<NoRoute>` can drive the same registry without mounting views.
The registry accepts at most 256 commands; registration freezes each descriptor.
The derive requires a concrete struct and a 1–128 byte ID without whitespace;
unknown/duplicate command options fail compilation. Named, tuple, newtype and unit
structs are supported. `output` defaults to `()`; `capability` defaults to
`RequiresWindow`, so headless execution needs an explicit declaration.

Generated descriptors contain self-contained Draft 2020-12 schemas with local
`$defs`/`$ref` references. Arguments describe **deserialization**; results describe
**serialization**. Serde renames, defaults, optional fields and skipped fields
therefore follow the direction actually used at that boundary. Result types must
implement Serde serialization/deserialization and JSON Schema. For custom codecs,
use Schemars annotations through `data(schema)` / `rake::schemars`, or implement
`ApplicationCommand` manually with honest schemas. Custom schema constraints are
descriptive, not an extra runtime validator. Validate domain rules in the handler
or a validating Deserialize implementation. Argument codecs must round-trip for
typed GUI dispatch; asymmetric/custom Serde conversions remain the author's
responsibility. Unknown fields follow Serde policy; add `deny_unknown_fields`
when they should fail instead of being ignored.

`commands list` returns the registered argument/result schemas and capability.
The Console panel uses those same schemas for editable JSON templates, including
local references and tuple/unit inputs; templates are not schema validators.
The terminal equivalent of the GUI call is:

```sh
rake ctl commands list --json
rake ctl commands run document.describe --args '{"document":"A"}' --request-key request-1 --json
```

`PageContext::dispatch`, `Navigator::dispatch`, `RuntimeClient::dispatch` and the
CLI all lower to the same `Command::Run` request, retry key and operation registry.
The derive supplies the command ID without rebuilding schemas on each dispatch.
Typed arguments decode before admission, and handlers start only when the driver
polls them. `RequiresWindow` fails before handler invocation when no window is
mounted; `Headless` describes requirements and does not itself launch a headless
process. The Web bridge grants no extra native capabilities.

Operation IDs, queued/running/terminal status, cancellation, failure, result
retention and retry rules are identical for GUI and CLI calls. Results are retained
as JSON in `OperationRecord.result`; decode with the declared output type where
its Serde contract permits. Application handlers do not hold a router scheduling
lane, so a handler can await navigation without deadlocking itself. Handlers own
model transaction rules. A committed command means successful handler return;
it does not assert presentation or data readiness, nor roll back external side
effects when cancellation wins. No view or borrowed GPUI context crosses await.

## Watches and transport bounds

`RuntimeClient::watch()` captures an owned snapshot and revision on the local event
loop. `next()` checks that revision before waiting, so changes before the first
poll are observed. `Query::Events` and `rake ctl events --follow` expose the same
runtime/epoch/revision cursor. This first adapter coalesces state and retains no
intermediate mutation replay: a skipped revision, stale epoch or other runtime
returns `Gap` plus a fresh authoritative snapshot. An unchanged poll contains no
snapshot. Consumers replace their state on Snapshot, Changed or Gap. Tracing loss
never defines authoritative routing or operation state.

`logs follow`, `requests list`, `traces export` and raw Diagnostics queries expose
collector sequence numbers and cumulative dropped/output-dropped counts. They are
best-effort diagnostics and have separate retention from state events.

Transport frames are limited to 1 MiB. Native IPC admits 32 active connections,
one message per connection, with a five-second I/O deadline. The broker admits
32 browser instances, 128 queued requests overall, at most 32 per instance and
64 simultaneous HTTP handlers. Undelivered/reply-lost requests may be redelivered
with the same request key. Client observation times out after ten seconds without
cancelling the queued command; tickets expire after 60 seconds. Browser leases
expire after 30 seconds without polling, requiring reconnect. Oversized snapshots
fail transport framing; this initial implementation has no pagination. A browser
may throttle or freeze polling when hidden; reconnect/retry preserves the same
RuntimeId while that application instance survives.

The [development supervisor](development.md), [build/packaging commands](packaging.md), reload boundary and
graphical inspector use this control surface. Project packaging runs without a live
runtime and enables no development transport in its Web output. Application commands
use the same registry, including generated metadata.

## Verification

```sh
cargo test -p rake --no-default-features --test runtime_control --test application_commands
cargo test -p rake-cli
cargo build -p rake-cli
python3 scripts/test-dev-bridge.py
```

The native integration test uses real Unix sockets and real terminal child processes,
including disconnect/retry and private discovery. The broker checks use real
loopback HTTP and the compiled CLI, including token, Host/Origin validation,
redelivery, queue bounds and lease expiry. The Wasm browser smoke check opened a
Live tab from the CLI, retried without creating a duplicate, edited the GUI draft,
resolved Save through the CLI and observed the committed close and task cleanup.
