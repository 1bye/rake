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
24 native frames per allocation and 256 symbolized code addresses per capture.
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
Sampler and query allocations on their own thread are excluded. A 256-slot atomic
queue defers contended frees without blocking. Monotonic tickets prevent an old
free from deleting a newer allocation at the same address. Queue overflow drops
uncertain observations and reports loss instead of retaining stale live samples.
Partial captures are not evidence of a leak. Native C/C++ allocator calls are not
intercepted.

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

Chrome's streaming loader requires V8 field order, including a final `strings`
array. The adapter serializes that order explicitly. Chrome also suspends Overlay
around each snapshot; this clears picker/highlight state without ending a memory
recording. Closing the whole session still releases both.

Use `rake.memory()` in Console for capture availability, sampled allocation counts,
actual sampled bytes, estimated bytes and omissions. `rake.stopMemory()` stops
background tracking and clears native profiles. Chrome's Stop button ends allocation
sampling. Instant heap captures keep tracking between snapshots for comparisons;
use the helper or close DevTools when finished. Clear all profiles only clears
Chrome's displayed captures. Console eager evaluation and syntax checking cannot
execute the stop helper.

Symbol-cache and call-tree admission favor the largest estimated allocation
samples, preserving their Rust call sites under bounded budgets. Each profile
admits at most 768 tree nodes and a 384 KiB tree budget, with 4096 stable stack
identities per recording. The summary exposes these limits. Freed-history evictions are reported
separately as `freedHistoryOmitted`; they count toward partial-capture warnings only
when freed history was requested. Stack depth and symbol limits can still leave
unresolved/deeper ancestors unavailable. Missing debug information stays unknown.

## Verification

Build `rake-cli`, then run `node scripts/test-memory-cdp.mjs` on a macOS GUI host
with Node 22 or newer. The fixture exercises real allocations, source links, stable
snapshot IDs, release/churn, Chrome's Overlay suspension, competing clients,
explicit stop, disconnect cleanup and checkpointed replacement. It never controls
Chrome. `--serve` leaves the example running and accepts `allocate`, `release`,
`churn`, `restart` and `quit` on stdin; paste its printed DevTools URL into Chrome.

Verified in stock Chrome 156.0.8078.74 on 2026-10-30:

- Allocation sampling Start/Stop, call-tree search for `allocate_probe`, and the
  source link opening `memory_fixture.rs` at line 23, column 26 in Sources.
- A sampling profile exported from Chrome and parsed as a native allocation
  profile. Requested JavaScript heap usage remains explicitly zero.
- Baseline/retained/released heap snapshots. Comparison showed eight new samples
  and +8.4 MB, followed by eight deletions and −8.4 MB after release.
- Heap snapshot export and reopening the exported file in Chrome (2.9 MB of
  estimated remaining sampled allocations in that capture).
- Console summary/stop helpers, clearing profiles on stop, and checkpointed
  replacement clearing old profiles before a new capture without reconnecting.

All 85 CLI unit tests, allocator lifecycle/failure/retention/contention tests, the public native
allocator consumer, doctests, formatting and warning-free Clippy checks passed.
Disabled-feature native builds and the supported GPUI/WASM configuration compile;
WASM does not expose the native allocator. A separate no-GPUI WASM check hits the
existing `control/web.rs` references to the GPUI-gated reload host; this task does
not change that configuration. Native execution was verified on macOS, not Linux.

A debug smoke measurement of 50,000 64-byte alloc/free pairs took 1,480 µs through
System, 2,179 µs with the feature/wrapper installed but dormant, and 26,610 µs while
recording at a 32 KiB interval. These are one local test run, not application-wide
performance estimates. Production omits the wrapper entirely; enabled profiling
has allocation-hook and stack-unwinding overhead.

## Coverage boundary

Chrome's JavaScript labels remain stock UI. **Allocations on timeline**, detached
DOM elements, JavaScript GC, arbitrary heap-object evaluation, full Rust ownership
and retainer graphs, complete type/object counts, process RSS, C/C++ allocations,
custom allocators bypassing the wrapper, and GPU/driver allocations are not exposed.
Use sampling and heap snapshots for the supported native workflows. Compare only
captures from the same process and recorder generation. Partial captures and
sampled growth identify investigation candidates, rather than proving a leak.
