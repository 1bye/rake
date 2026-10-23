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
internal application tab. NewWindow returns the router's unsupported-capability
error and is absent from the advertised supported targets.

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
responses and registered application commands. Queries expose runtime capabilities,
application state, routes, tabs/histories, tasks/owners, operations/results, pending
decisions and bounded diagnostics. Logs, request records and span-close exports
read the same collector; these are initial diagnostic records, not a graphical
inspector, complete request archive or full trace-tree export.

A submission returns an operation ID before commit. The terminal waits until a
terminal state or a pending decision by default. `--no-wait` returns immediately.
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

Registration currently uses `ApplicationCommand` with explicit schemas. The
proposed `Command` derive is not implemented. The registry limits registration to
256 stable command keys and rejects duplicates. It deserializes typed arguments
before admission and invokes handlers only when the operation driver polls them.
A `RequiresWindow` command fails before invoking its handler when run headlessly.
`Headless` describes the handler's requirements; it does not itself launch a
headless application process.

```rust
use rake::control::*;
use rake::router::{NoRoute, Router, RouterConfig};
use rake::serde_json::json;

#[rake::data]
#[serde(deny_unknown_fields)]
struct Describe { document: String }

impl ApplicationCommand for Describe {
    type Output = String;
    fn descriptor() -> CommandDescriptor {
        CommandDescriptor {
            id: "document.describe".into(),
            arguments: json!({"type":"object", "properties":{"document":{"type":"string"}},
                "required":["document"], "additionalProperties":false}),
            result: json!({"type":"string"}),
            capability: ExecutionCapability::Headless,
        }
    }
}

let commands = Commands::new();
commands.register::<Describe, _, _>(|arguments, context| async move {
    // context.app.tasks() provides application-owned work; context.cancellation
    // exposes this operation's signal. Services/models can be captured explicitly.
    Ok(format!("Document {}", arguments.document))
}).unwrap();

// GUI factories install this with App::commands(commands). PageContext::dispatch
// and Navigator::dispatch use the same registry without retaining the page.
let router = Router::<NoRoute>::new(RouterConfig::default());
router.client().commands().register::<Describe, _, _>(|args, _| async move {
    Ok(args.document)
}).unwrap();
let reply = router.client().dispatch(Describe { document: "A".into() }, "request-1".into()).unwrap();
assert!(matches!(reply, Reply::Submitted { .. }));
// The host must drive router.run(); observation does not execute the command.
```

The terminal equivalent is `rake ctl commands run document.describe --args
'{"document":"A"}'`. Application handlers do not hold a router scheduling lane,
so a handler can await a normal navigation operation without deadlocking itself.
They own their model transaction rules. A committed command means its handler
returned successfully; it does not assert presentation or data readiness.

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

The project supervisor, build/package commands, element inspector, reload adapters,
generated command derive and richer diagnostics are later work. The present CLI
reports unsupported commands instead of pretending those capabilities exist.

## Verification

```sh
cargo test -p rake --no-default-features --test runtime_control
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
