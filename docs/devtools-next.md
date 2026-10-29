# DevTools implementation plan

Five changes, each delivered in a separate commit:

1. **Implemented.** Larger trees: retain a bounded larger capture, page the tree index, load element
   details on demand, and preserve identities/expansion across incremental updates.
2. **Implemented for captured boundaries.** Picking and geometry: use observed paint order, validate clipping and offsets,
   and document exact GPUI hook requirements for unavailable used layout metrics.
3. **Implemented.** Live styles: temporary validated overrides for colors, spacing, dimensions and
   typography, with declaration toggles, reset and undo; never rewrite Rust source.
4. **Implemented.** Frame performance: bounded live recording, frame duration and inspected-node
   layout/prepaint/paint timings linked to Rust source; distinguish CPU wall time
   from GPU execution and presentation latency.
5. **Implemented for captured boundaries.** Accessibility and interaction: roles, accessible names, focus state/order and
   explicitly registered handler metadata, without pretending to enumerate private
   GPUI listener closures.

Each phase updates coverage documentation and exercises its protocol, runtime and
frontend behavior. Native tests and explicit WASM checks remain separate evidence.

## Verification

Native runtime, protocol, macro, inspector and CDP suites pass, including real
local IPC/HTTP tests and GPUI TestPlatform layout, focus and draw tests. Inspector
JavaScript model/panel tests and authenticated bridge tests pass. Native Clippy
(including no-default-features), formatting, rustdoc warnings and explicit WASM
library checks pass. Final regression checks cover frame lease expiry without a
window draw, frame attribution after pending metadata eviction, and editing across
stable tree updates.

A real GPU/browser UI end-to-end run was not performed for this batch. Coverage
limits and GPUI hook requirements remain documented in element-inspection.md and
devtools.md; the implementation does not claim full platform accessibility,
private layout metrics or GPU execution profiling.

## Elements completion batch

Each step is a separate commit. Existing coverage and GPUI hook limits remain explicit.

1. **Implemented:** broaden temporary editing to flex/grid, alignment, overflow, position and borders; validate against real GPUI layout.
2. **Implemented:** open captured style attributes and handler registration sites from both frontends and CDP.
3. **Implemented:** improve accessible names and states, including captured text.
4. **Pending:** tree navigation and context actions, including copying text/paths and expanding/collapsing branches.
5. **Pending:** real Chrome checks, regression suites and a current Elements coverage checklist.
