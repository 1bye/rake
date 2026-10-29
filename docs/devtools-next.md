# DevTools implementation plan

Five changes, each delivered in a separate commit:

1. **Implemented.** Larger trees: retain a bounded larger capture, page the tree index, load element
   details on demand, and preserve identities/expansion across incremental updates.
2. Picking and geometry: use observed paint order, validate clipping and offsets,
   and document exact GPUI hook requirements for unavailable used layout metrics.
3. Live styles: temporary validated overrides for colors, spacing, dimensions and
   typography, with declaration toggles, reset and undo; never rewrite Rust source.
4. Frame performance: bounded live recording, frame duration and inspected-node
   layout/prepaint/paint timings linked to Rust source; distinguish CPU wall time
   from GPU execution and presentation latency.
5. Accessibility and interaction: roles, accessible names, focus state/order and
   explicitly registered handler metadata, without pretending to enumerate private
   GPUI listener closures.

Each phase updates coverage documentation and exercises its protocol, runtime and
frontend behavior. Native tests and explicit WASM checks remain separate evidence.
