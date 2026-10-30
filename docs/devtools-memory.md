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
