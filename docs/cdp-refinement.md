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
