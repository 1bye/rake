# Chrome Memory

The native adapter uses Chrome's `HeapProfiler` sampling profile and heap snapshot
formats. A heap snapshot is an explicitly labeled projection of sampled live Rust
allocator blocks, rooted in a synthetic allocation catalog. Its edges do not
describe Rust ownership. Allocation counts are sampled counts; byte totals are
inverse-probability estimates, not RSS, exact heap size, or GPU memory.

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
24 native frames per allocation and 256 symbolized code addresses per recording.
Omissions are explicit. Snapshot identities survive repeated captures in one
generation and expire on a new recording/runtime. Source availability depends on
native debug information and the existing workspace source confinement.

Enable `rake/devtools-memory` only in development. On Unix, install
`rake::memory::TrackingAllocator::new(existing_allocator)` as the executable's
`#[global_allocator]`, behind the application's own development feature. The
library never selects a global allocator for its consumers. Without the feature
and gated static, production uses its original allocator with no tracking hooks,
recorder state or stack unwinding dependency. WASM does not advertise native
memory capture.

`record_memory` controls one process-wide recorder with a unique client token
and a renewable lease of at most five seconds. A competing client cannot stop or
renew it. Expired recording tokens cannot silently start a fresh capture.
`memory` returns that owner's bounded samples. Allocation hooks delegate to the
wrapped allocator, including zeroed and failed/reallocated blocks. They suppress
recursive instrumentation, use nonblocking table access and count admission loss.
Debug information is resolved only when querying, outside the hook/table lock.
Sampler and query allocations on their own thread are excluded. Freed blocks
missed through contention can leave stale samples, so a partial capture is not
evidence of a leak. Native C/C++ allocator calls are not intercepted.

Chrome's **Allocation sampling** uses `HeapProfiler.startSampling`,
`getSamplingProfile` and `stopSampling`. Call-tree IDs stay stable across polls of
one recording. Rows carry demangled native function names and captured Rust source
links where debug information is available. Recently freed samples are included
when Chrome requests either collected-object option; Rust has no minor/major GC.
Profiles expose bounded measurement/count/loss metadata in their `rake` field and
through `Rake.getMemorySummary`. Chrome's JavaScript heap usage remains zero;
that number does not measure the native heap. Garbage-collection requests fail
explicitly. Closing the connection, disabling HeapProfiler or replacing the native
runtime stops the recorder; the lease also expires after client failure.

The routing example's `memory-test` feature installs the wrapper in its native
executable. Its typed `test.memory({"action":"allocate"})` operation retains eight
1 MiB blocks; `release` drops them, and `churn` allocates/drops eight temporary blocks.
At most sixteen fixture blocks can be retained. These operations are absent from
ordinary example builds.

**Heap snapshot** starts native tracking if necessary and projects its sampled live
blocks into Chrome's snapshot format. Repeated captures within that recording use
the same allocation IDs. **Comparison** can show new and released sample counts
and estimated byte deltas. Actual requested bytes remain in the export's `rake`
metadata. Only allocations observed after recording starts are eligible;
pre-existing allocations are not retroactively discovered. Synthetic root edges
serve Chrome's viewer, and retained-size/retainer views do not represent Rust
reference ownership. Heap snapshot nodes group by the first captured workspace
allocation caller; unresolved callers stay explicitly unknown. Sampling profiles
provide the full captured native stack/source links.

Snapshot JSON is capped at 512 KiB and streamed in UTF-8-safe chunks. The complete
event batch must fit the connection's outbound budget before any chunk is sent.
Replacement emits `HeapProfiler.resetProfiles` so the next runtime cannot be
compared to stale allocation identities. Exported files remain offline captures.
