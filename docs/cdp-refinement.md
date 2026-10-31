# CDP refinement

The five changes preserve Chrome's JSON/WebSocket contract and the existing
production feature gate. Their implementation order is:

1. Cheap runtime index revisions and bounded immutable sharing across clients.
2. Responsive socket handling, bounded runtime work, total deadlines and explicit shutdown.
3. Encode-once event delivery, batched writes and reduced projection copies.
4. Typed command validation, structured errors and a pinned compatibility contract.
5. Repeatable load, reconnect and slow-client checks with before/after measurements.

`inspect_revision` is an additive, capability-advertised runtime query. Older
runtimes retain paged/full snapshot fallback. An unchanged index skips page reads
and DOM diffing; the selected element's live styles still refresh independently.
The optional metadata revision invalidates live accessibility without changing
control capture IDs, so focus changes do not expire in-flight edits. Cache keys
include both identities; older hosts can omit the additive metadata field.
Revision skipping and shared caching require paged indexes. Full-capture fallback
continues polling complete snapshots because they also carry live style values.
The shared index cache retains at most four runtimes and 32 MiB of serialized
metadata. Source grants, selected nodes, object handles, editing and cursors stay
connection-local. Simultaneous cold misses may read the same capture; no socket
I/O occurs under the cache lock.

Socket I/O has one owner. A connection admits up to 64 pending requests within
a 4 MiB encoded-request budget and two encoded outbound messages in addition to
the existing session byte budget. Its ordered
worker performs runtime operations, while ping, close and stateless startup
acknowledgements remain responsive. Queued requests expire before execution;
executing operations share a two-second blocking-exchange deadline. Mutations
with uncertain outcomes are never replayed. Disconnect cancels queued work.
Explicit worker shutdown gets one 100 ms cleanup budget; inspection leases cover lost
cleanup acknowledgements. Dropping session state performs no socket I/O.

Projection nodes now own shared views into immutable captures. Only hydrated
nodes copy their details when a new index arrives. Events are bounded while
encoding and retained as wire text, so preflight and delivery reuse the same
encoding. Responses stop encoding at 1 MiB while preserving mutation retry keys.
Cache byte accounting uses a counting writer. The socket owner flushes at most
32 queued messages together before returning to input.

## Compatibility contract

The tested subset is pinned to [CDP protocol r1695952](https://github.com/ChromeDevTools/devtools-protocol/tree/9b417d8a187ae7ecff367d9a9741d564658029b2)
(version 1.3). `crates/rake-cli/src/cdp/contract.json` contains the upstream
field/type definitions used by contract tests, with descriptions removed.
It is a compatibility reference, not a claim of full protocol implementation.

Common DOM parameters decode through borrowed, typed structures before runtime
I/O. Positive checked CDP node IDs stay distinct from opaque runtime IDs. Depth
and search ranges reject invalid numeric values; newer optional fields remain
accepted. Enabled domains are an enum. Recording expiry/restart uses typed error
categories while preserving the CDP error payload. `Browser.getVersion` and
`Schema.getDomains` expose this native adapter's identity and domain subset.
Unsupported methods still return -32601, including JavaScript debugging.

When updating the pin, refresh the extracted definitions from the upstream JSON,
run parameter/return contract tests and the stock Chrome workflow checks. The
locally installed showcase browser is Chrome 156.0.8078.74; a browser version is
not proof that every domain is implemented.

## Baseline

Run the generated-input projection benchmark explicitly:

```sh
cargo test -p rake-cli --release --offline --bin rake cdp_projection_baseline -- --ignored --nocapture --test-threads=1
```

The test allocator counts allocation calls and requested bytes on the benchmark
thread only. Inputs are prepared outside the measured interval. Results cover
25 unchanged index projections, not native GPU or Chrome performance.

Before refinement on 2026-10-31:

| Nodes | Median µs | p95 µs | Allocations/update | Requested bytes/update |
| --- | ---: | ---: | ---: | ---: |
| 1,000 | 1,513 | 1,804 | 13,052 | 2,276,540 |
| 8,192 | 15,382 | 16,398 | 106,603 | 18,797,756 |

## Results and verification

The same release projection benchmark after the ownership change:

| Nodes | Median µs | p95 µs | Allocations/update | Requested bytes/update |
| --- | ---: | ---: | ---: | ---: |
| 1,000 | 1,086 | 1,292 | 3,055 | 611,314 |
| 8,192 | 13,675 | 18,804 | 24,686 | 5,125,938 |

At 8,192 nodes this removes 77% of allocation calls and 73% of requested bytes.
Projection latency remains workload-sensitive; p95 did not improve in this run.
The normal unchanged-revision path skips projection entirely.

Measure that path through real private Unix sockets:

```sh
cargo test -p rake-cli --release --offline --bin rake cdp_refresh_load_benchmark -- --ignored --nocapture --test-threads=1
```

This compares the same implementation with revision negotiation disabled/enabled,
using generated 8,192-node captures, 32 pages, one cold read per client and 25 warm
refreshes per client. Four sessions share the immutable cache and are refreshed
sequentially. Timings/allocations below describe one warm refresh on the client
thread. Query counts and serialized request/reply bytes cover the entire run,
including cold loads; they exclude TCP/WebSocket framing and server allocations.

| Clients | Revisions | Warm median µs | Warm p95 µs | Allocations/refresh | Requested bytes/refresh | IPC queries | IPC bytes |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | disabled | 29,113 | 33,822 | 66,948 | 32,516,595 | 832 | 77,544,298 |
| 1 | enabled | 48 | 89 | 15 | 9,599 | 58 | 2,988,323 |
| 4 | disabled | 26,871 | 29,209 | 66,951 | 32,516,787 | 3,328 | 310,177,192 |
| 4 | enabled | 47 | 104 | 15 | 9,596 | 136 | 3,005,873 |

Cold loading all four clients took 87,298 µs with negotiation disabled and
36,469 µs enabled. These are generated-input adapter/IPC measurements, not claims
about native frame rates, GPU rendering, or Chrome UI latency. Simultaneous cold
cache misses can still duplicate reads.

Default regression tests now cover eight concurrent clients reconnecting (256
ordered requests), final buffered reply delivery, ping/startup responsiveness
under blocked work, bounded admission and disconnect cancellation, a nonreading
peer with write timeout and joined worker, and cache isolation/replacement/count
and byte eviction. Tests use real loopback sockets and controlled workers.

Paged captures are installed only after contiguous pages reach their declared
total. Skipped/truncated pages, changed totals/status/generations and expired
captures fail without changing the frontend tree. A subsequent read can recover;
uncertain mutations are not retried. The WebSocket write buffer is capped at 2 MiB, in
addition to session and worker queue limits. A stopped worker's final batch is
flushed before closing the connection.

Relevant checks:

```sh
cargo test -p rake-cli -p rake-client -p rake-protocol --offline
cargo clippy -p rake-cli -p rake-client -p rake-protocol --all-targets --offline -- -D warnings
cargo test -p rake --no-default-features --features devtools --lib inspection::tests:: --offline
cargo check -p rake --no-default-features --offline
cargo check -p rake-client -p rake-protocol --target wasm32-unknown-unknown --offline
cargo build -p rake-cli -p rake-view --features rake/devtools --offline
node scripts/test-elements-cdp.mjs
node scripts/test-console-cdp.mjs
node scripts/test-network-cdp.mjs
node scripts/test-performance-cdp.mjs
```

All four native workflow scripts passed on 2026-10-31. They require a real macOS
GUI and exercise the native app/CDP through WebSocket; they do not automate stock
Chrome's interface. Run them sequentially because reload fixtures watch the same
source workspace.

The Console run caught a deadline interaction: command submission could succeed
but continued observation would reach the enclosing operation deadline and show
an exception. Evaluation now stops polling within that budget and returns the
last observed pending state with its runtime-scoped observation token and retry
key. A late observation timeout includes a warning; it never resubmits. The
socket regression verifies a single submission for both lost acknowledgements
and pending outcomes. `current_deadline()` exposes the enclosing synchronous
scope so callers can budget polling as well as I/O.

Console and Performance source-access checks were updated to request an actually
uncaptured file URL. Their old numeric-URL replacements left current workspace
URLs unchanged and incorrectly fetched the original allowed source.

## Native temporary-edit redraws

The standalone inspection host must notify its content entity as well as its
wrapper. GPUI tracks that erased content entity as a render dependency; notifying
only the wrapper could acknowledge an edit without scheduling the native draw
that publishes the edited text, geometry and styles. The host now invalidates
both entities through GPUI's public API. The production feature gate is unchanged.

A draw can also land between the adapter's capture lookup and runtime admission.
For text/style editing, reset and undo, an explicit `StaleGeneration` rejection
allows one fresh lookup and submission using the retained CDP identity. That
rejection occurs before mutation. Removed elements are never retargeted, repeated
staleness remains an error, and transport failures or other rejections never cause
another submission. All attempts share the enclosing operation deadline.

Once a style mutation is acknowledged, its response no longer depends on a
second capture/detail read. Such a read could expire during redraw and report
an error even though reset or editing had already succeeded. The adapter stages
acknowledged declarations for Chrome's immediate style/range response; normal
polling publishes rendered geometry and the runtime-owned undo state. A socket
regression closes the host immediately after committing: the acknowledgement and
edited CSS still succeed. It fails on the former post-mutation read path.

The standalone regression fails on the old invalidation path and checks Unicode,
empty text, actual GPUI layout, reset and undo with the GPUI TestPlatform. Separate
real Unix-socket tests cover remapping, removal, bounded recovery and other
rejections, including a lost edit acknowledgement with no recovery request.
Neither test category claims GPU or Chrome UI execution. Five consecutive native
Elements workflow runs passed on 2026-10-31 after both fixes, covering actual
GPUI rendering, Unicode/empty text, relayout, style edits/toggles, reset and undo.

The final rebuilt adapter was verified in stock Chrome **156.0.8078.74** on
2026-10-31. Inline editing changed a paragraph to `Verified native text — café 🙂`;
`rake.resetText($0)` restored `Count: 0`, and `rake.undoText($0)` restored the edit.
Editing the button's override in Styles set its width to 260px;
`rake.resetStyles($0)` restored its original measured 109px, and
`rake.undoStyles($0)` restored both the override and measured 260px width.
All four Console helpers returned successfully, with no Console errors. A direct
native capture confirmed the final Unicode text, declaration and rendered width.

Native assertions wait for the complete rendered edit, including border and
overflow. Width alone could match a previous frame before the newly acknowledged
declarations had drawn; an acknowledgement does not claim a synchronous frame.

Stock Chrome verification also exposed a startup admission regression: its burst
of roughly 40 small requests exceeded the old four-request queue and rejected
`Runtime.enable`, leaving Console without an execution context. Admission now
has independent count and byte budgets so that normal startup fits without
allowing 64 full-sized requests. A deterministic WebSocket test holds the worker
until the recorded startup burst is admitted. Separate overload cases verify
both limits while ping, startup acknowledgements and close remain responsive.

For manual stock Chrome verification, start a dedicated native fixture:

```sh
cargo build -p rake-cli -p rake-view --features rake/devtools --offline
node scripts/test-elements-cdp.mjs --serve
```

Open the printed `devtoolsFrontendUrl` in Chrome. Ctrl+C stops that fixture and
cleans up its app, adapter and temporary control directory. Without `--serve`,
the script runs the native protocol assertions and cleans up automatically.
Evaluation helpers now assert `exceptionDetails` immediately instead of masking
an exception as an eventual redraw timeout; protocol failures name their method.
