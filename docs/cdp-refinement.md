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

Socket I/O has one owner. A connection admits four queued requests and two encoded
outbound messages in addition to the existing session byte budget. Its ordered
worker performs runtime operations, while ping, close and stateless startup
acknowledgements remain responsive. Queued requests expire before execution;
executing operations share a two-second blocking-exchange deadline. Mutations
are never replayed by the transport. Disconnect cancels queued work. Explicit
worker shutdown gets one 100 ms cleanup budget; inspection leases cover lost
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
mutations are not retried. The WebSocket write buffer is capped at 2 MiB, in
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
