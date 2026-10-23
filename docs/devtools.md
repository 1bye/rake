# Graphical development tools

Build the CLI with `cargo build -p rake-cli`, then run:

```sh
target/debug/rake dev --target desktop --package rake-routing
target/debug/rake dev --target web --package rake-routing --port 8081
target/debug/rake inspect --session /tmp/rake-dev-EXAMPLE
```

Native dev opens the inspector. Web dev opens the application; its development
badge links to Devtools in a separate tab. Open the private launch URL printed
by the command when using `--no-open`. `rake inspect` connects to existing opt-in
runtimes in the normal private control directory or the selected session. It
accepts `--instance UUID|APPLICATION_ID`, `--port`, `--no-open` and `--json`.
Python 3.11+ is required; the CLI bundles the inspector assets and server helpers.

The browser UI belongs to the development host, so it survives application
crashes, process restarts and full Web page reloads. Runtime selection is explicit
when several instances match. A checkpointed successor advertises `restored_from`;
the inspector follows that exact predecessor, clears all old state and diagnostic
cursors, and uses the replacement's fresh RuntimeId. Logical tab IDs may survive
restoration; operation IDs and retry keys never cross runtimes.

## Panels

| Panel | Inspection and actions |
| --- | --- |
| Workspace | Logical window, selection, host visibility, tab policies, destinations, full retained histories/cursors, entry revisions and dirtiness; select, Back, Forward, close and change policy. |
| Routes | Forms generated from registered typed destination descriptors; open/reveal or navigate/replace in an exact tab, with independent work policy and activation options. |
| Tasks | Lifetime owner separately from causal origin; task kind, eligibility, state, RunId and generation; cancel and follow related diagnostics. Explicit application progress is unavailable unless logged. |
| Operations | Queued/running/awaiting-decision/terminal states, retained results and errors; inspect or cancel through the shared dispatcher. Commit does not imply finished data loading. |
| Decisions | Pending guard messages and advertised choices. Responses use the same revision checks as GUI/CLI responses; obsolete choices cannot approve changed state. |
| Console | Structured tracing severity/fields/context, bounded build and native stdout/stderr, registered typed commands with argument/result schemas and JSON input. |
| Requests | Method, redacted URL, status, elapsed time, bytes consumed and observed lifecycle events; distinguish headers, completion, cancellation/failure and abandonment. |
| Traces | Collector-local span identities and parent links, inherited fields, elapsed wall-time timeline and related tasks/requests/logs. Missing ancestry or closes remain explicit. |
| Reload | Supervisor build/reload/error status and the runtime lifecycle boundary, generation and restoration origin. |

Search, context filters, paging and JSON diagnostic export operate on retained
observations. Related links only use actual recorded context. Process text has
its own supervisor elapsed clock and no inferred runtime/tab ownership. Structured
tracing can appear again in process stderr; these remain distinct sources.
Legacy `log` forwarding requires the embedding host's tracing bridge. Arbitrary
browser console output and arbitrary Rust evaluation are not provided. The routing
example registers `demo.echo` and `demo.fetch` for explicit command/log/HTTP checks.

## One authoritative control surface

All runtime reads and actions use protocol-1 `Request`/`Reply` envelopes through
`RuntimeClient`: the same queries, commands and event cursors as the terminal.
The host only proxies private native IPC or the authenticated Web broker. It
does not mutate router state or approve decisions. Browser snapshots come from
Events, never reconstruction from logs. Revision gaps replace state with a fresh
authoritative snapshot and increment the visible resynchronization count.

Every mutation has a stable request key. Submission receipt, operation commit and
decision acknowledgement remain distinct. An uncertain delivery offers a retry
of the exact original envelope/key, scoped to its original runtime. A timeout,
disconnect, runtime switch or Pause updates never cancels work or chooses a guard
response. JavaScript forms reject unsafe integer inputs/identities; the typed Rust
client and CLI can represent larger integers.

The UI polls serially, observes bounded diagnostics incrementally, retains at most
1,000 records/2 MiB and renders at most 100 rows per page. It displays runtime
collector/output drops, inspector evictions, missing diagnostic sequences and
state resynchronizations. Capture is best effort: these bounds and transport's
1 MiB frame limit can leave incomplete trees or request lifecycles. Process output
retains 128 records of at most 2 KiB each, with truncation/drop indicators. Nothing
in these buffers defines authoritative task or operation state.

## Capture and access boundaries

Rake HTTP and the native GPUI HTTP adapter are captured. Browser GPUI-internal
Fetch and third-party clients need their own adapters. Authorization/cookie headers,
bodies, URL credentials, queries and fragments are excluded. Request metadata and
causal context remain attached through body completion or abandonment, even if
consumption occurs under another task. DNS/TLS and browser-hidden timings are
unavailable; span durations measure elapsed wall time, not CPU usage.

Control remains opt-in and development-only. The loopback server validates
Host/Origin and requires the private session token. The launch fragment is removed
and stored in that tab's session storage; public assets contain no credentials.
Keep private launch URLs and diagnostic exports private. An inspector-only native
origin serves no application files, validates owner-only discovery/socket paths,
and forwards no arbitrary filesystem or process commands. Web application origins
can inspect Web runtimes only; they cannot proxy native IPC. Production builds
contain no inspector connection metadata.

The element/GPU inspector, CPU profiler and native code loader are outside this
stage. Native IPC currently requires Unix. Native replacement continues to use
the external reloader boundary or the explicitly reported checkpointed process
restart described in [development](development.md).
