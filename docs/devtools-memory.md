# Chrome Memory

The native adapter uses Chrome's `HeapProfiler` sampling profile and heap snapshot formats.

- A heap snapshot is a labeled projection of sampled live Rust allocator blocks, rooted in a synthetic allocation catalog. Its edges don't describe Rust ownership.
- Allocation counts are sampled counts.
- Byte totals are inverse-probability estimates, not RSS, exact heap size or GPU memory.

Delivery has five separate commits:

1. Define bounded native measurements and Chrome format compatibility.
2. Add an opt-in native allocator wrapper and leased runtime capture.
3. Connect Memory sampling and Rust allocation call sites.
4. Add live-allocation snapshots for before/after comparisons.
5. Verify stock Chrome capture/export/cleanup/replacement and record limitations.

Chrome's formats are documented in the upstream
[HeapProfiler protocol](https://github.com/ChromeDevTools/devtools-protocol/blob/master/json/js_protocol.json)
and consumed by its
[allocation sampling view](https://github.com/ChromeDevTools/devtools-frontend/blob/main/front_end/panels/profiler/HeapProfileView.ts).
Rust's [global allocator contract](https://doc.rust-lang.org/std/alloc/trait.GlobalAlloc.html)
requires non-unwinding, reentrancy-safe hooks. The recorder must never dereference
application blocks, allocate recursively in a hook, or block a hook on capture.

The representation retains at most 512 live and 512 recently freed samples,
24 native frames per allocation and 256 symbolized code addresses per capture.
Omissions are explicit. Snapshot identities survive repeated captures in one
generation and expire on a new recording/runtime. Source availability depends on
native debug information and the existing workspace source confinement.

Enable `rake/devtools-memory` only in development. On Unix, install `rake::memory::TrackingAllocator::new(existing_allocator)` as your executable's `#[global_allocator]`, behind your own development feature. The library never selects a global allocator for you.

Without the feature and gated static, production uses its original allocator with no hooks, recorder state or unwinding dependency. WASM doesn't advertise native memory capture.

`record_memory` controls one process-wide recorder with a unique client token and a renewable lease of at most five seconds. A competing client can't stop or renew it, and expired tokens can't silently start a fresh capture. `memory` returns that owner's bounded samples.

How the hooks behave:

- They delegate to the wrapped allocator, including zeroed and failed or reallocated blocks.
- They suppress recursive instrumentation, use nonblocking table access and count admission loss.
- Debug information is resolved only when querying, outside the hook lock. Sampler and query allocations on their own thread are excluded.
- A 256-slot atomic queue defers contended frees, and monotonic tickets stop an old free deleting a newer allocation at the same address.
- Queue overflow drops uncertain observations and reports loss, rather than keeping stale samples.

Partial captures aren't evidence of a leak. Native C and C++ allocator calls aren't intercepted.

Chrome's **Allocation sampling** uses `HeapProfiler.startSampling`, `getSamplingProfile` and `stopSampling`. Call-tree IDs stay stable across polls of one recording, and rows carry demangled function names and Rust source links where debug information exists.

- Recently freed samples appear when Chrome requests either collected-object option. Rust has no minor or major GC, so garbage-collection requests fail explicitly.
- Profiles expose measurement, count and loss metadata in their `rake` field and through `Rake.getMemorySummary`.
- Chrome's JavaScript heap usage stays zero, which doesn't measure the native heap.
- Closing the connection, disabling HeapProfiler or replacing the runtime stops the recorder. The lease also expires after client failure.

The routing example's `memory-test` feature installs the wrapper in its native
executable. Its typed `test.memory({"action":"allocate"})` operation retains eight
1 MiB blocks; `release` drops them, and `churn` allocates/drops eight temporary blocks.
At most sixteen fixture blocks can be retained. These operations are absent from
ordinary example builds.

**Heap snapshot** starts native tracking if needed and projects sampled live blocks into Chrome's format. Repeated captures in one recording use the same allocation IDs, and **Comparison** can show new and released sample counts and estimated byte deltas. Actual requested bytes stay in the export's `rake` metadata.

- Only allocations observed after recording starts are eligible.
- Synthetic root edges serve Chrome's viewer, so retained-size and retainer views don't represent Rust ownership.
- Nodes group by the first captured workspace caller, and unresolved callers stay explicitly unknown. Sampling profiles give the full stack and source links.

Snapshot JSON is capped at 512 KiB and streamed in UTF-8-safe chunks. The complete
event batch must fit the connection's outbound budget before any chunk is sent.
Replacement emits `HeapProfiler.resetProfiles` so the next runtime cannot be
compared to stale allocation identities. Exported files remain offline captures.

Chrome's streaming loader requires V8 field order, including a final `strings`
array. The adapter serializes that order explicitly. Chrome also suspends Overlay
around each snapshot; this clears picker/highlight state without ending a memory
recording. Closing the whole session still releases both.

- `rake.memory()` in Console shows capture availability, sampled counts, actual and estimated bytes, and omissions.
- `rake.stopMemory()` stops background tracking and clears native profiles.
- Chrome's Stop button ends allocation sampling.
- Instant heap captures keep tracking between snapshots for comparisons, so use the helper or close DevTools when finished.
- Clear all profiles only clears Chrome's displayed captures.

Console eager evaluation and syntax checking can't run the stop helper.

Symbol-cache and call-tree admission favor the largest estimated samples, keeping their Rust call sites under bounded budgets. Each profile admits at most 768 tree nodes and a 384 KiB tree budget, with 4096 stable stack identities per recording, and the summary shows these limits.

Freed-history evictions are reported as `freedHistoryOmitted`, and count toward partial-capture warnings only when freed history was requested. Stack depth and symbol limits can still leave ancestors unavailable, and missing debug information stays unknown.

## Verification

Build `rake-cli`, then run `node scripts/test-memory-cdp.mjs` on a macOS GUI host with Node 22 or newer. It exercises real allocations, source links, stable snapshot IDs, release and churn, Chrome's Overlay suspension, competing clients, explicit stop, disconnect cleanup and checkpointed replacement. It never controls Chrome.

For a manual pass, `--serve` keeps the example running and accepts `allocate`, `release`, `churn`, `restart` and `quit` on stdin. Paste its printed DevTools URL into Chrome.

Verified in stock Chrome 156.0.8078.74 on 2026-10-30:

See [test coverage](internal/test-coverage.md) for what's been checked in stock Chrome. Native execution was verified on macOS, not Linux.

A debug smoke measurement of 50,000 64-byte alloc/free pairs took 1,480 µs through
System, 2,179 µs with the feature/wrapper installed but dormant, and 26,610 µs while
recording at a 32 KiB interval. These are one local test run, not application-wide
performance estimates. Production omits the wrapper entirely; enabled profiling
has allocation-hook and stack-unwinding overhead.

## Coverage boundary

Chrome's JavaScript labels remain stock UI. Use sampling and heap snapshots for the supported workflows.

**Not exposed:**

- Allocations on timeline, detached DOM elements, JavaScript GC and arbitrary heap-object evaluation.
- Full Rust ownership and retainer graphs, and complete type or object counts.
- Process RSS, C and C++ allocations, custom allocators bypassing the wrapper, and GPU or driver allocations.

Compare only captures from the same process and recorder generation. Partial captures and sampled growth point to things worth investigating, and don't prove a leak.
