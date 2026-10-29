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

A real GPU/browser UI end-to-end run was not performed for the original five-change batch above. The later Elements completion evidence is recorded below. Coverage
limits and GPUI hook requirements remain documented in element-inspection.md and
devtools.md; the implementation does not claim full platform accessibility,
private layout metrics or GPU execution profiling.

## Elements completion batch

Each step is a separate commit. Existing coverage and GPUI hook limits remain explicit.

1. **Implemented:** broaden temporary editing to flex/grid, alignment, overflow, position and borders; validate against real GPUI layout.
2. **Implemented:** open captured style attributes and handler registration sites from both frontends and CDP.
3. **Implemented:** improve accessible names and states, including captured text.
4. **Implemented:** tree navigation and context actions, including copying text/paths and expanding/collapsing branches.
5. **Implemented:** live native/CDP regression harness, HTTP-browser checks, source-detail hydration and click/visibility regressions, plus the coverage checklist below. Stock Chrome interaction verification remains limited as described below.

### Elements coverage checklist

| Capability | Current coverage | Evidence |
| --- | --- | --- |
| Tree and text | Automatic `view!` and explicit boundaries; text/inline runs; stable identities; paged/lazy trees; incremental updates | GPUI TestPlatform, protocol/CDP suites, native `rake-view` and HTTP browser |
| Navigation | Find by name/ID/text, disclosure, keyboard navigation, focus, reveal and retained selection | Tree/model tests; live native focus and HTTP browser collapse/expand/search |
| Picking/highlight | Captured paint order and clipping; hidden branches excluded; leased pointer capture | GPUI input tests; hidden/disabled regression; native highlight commands |
| Styles | Live resolved values/hover; observed provenance; validated temporary flex/grid/alignment/overflow/position/border edits; reset/undo | Real GPUI layout tests; live native grid/border/width edits; HTTP browser edits/reset |
| Source | Construction, style-attribute and handler-registration sites; system default editor; workspace-confined paths | Source-resolution/host tests; paged-detail regression; source actions visible in browser |
| Accessibility | Explicit/text/content names; name origin; text roles; checked/pressed/expanded/selected and other captured states; focus order | GPUI name/state tests; CDP updates; native AX and HTTP browser name/state panes |
| Copy/context actions | Escaped synthetic markup, retained text, inspection paths; subtree actions; keyboard menus | Portable/JS tests; live native `DOM.getOuterHTML`; HTTP browser menus/copy/Escape |
| Input stability | Unrelated diagnostics do not remount Elements; pressed controls survive through click; toolbar remains reachable while scrolling | Before/after regression; live browser collapse/expand with updates enabled |

The completion batch was checked on macOS with the actual GPUI `rake-view` and
`rake-routing` windows. The native/CDP harness exercised 56 macro boundaries,
text previews, grid and border edits, reset/undo, node identity, focus, highlight,
source metadata and accessible names. The routed-window test exercised 29
boundaries, ownership/replacement, stale IDs and the real inspector HTTP proxy.
The HTTP inspector was exercised in an actual browser with live updates enabled:
collapse 56 nodes to one, expand again, search, name/state and source panes,
apply/reset a real width override, copy feedback and Escape menu dismissal.

Stock Chrome connected and its Elements/context-menu UI was observed. The browser
automation URL policy rejected opening `devtools://` in the automated browser, so
this batch does not establish a complete automated stock-Chrome interaction pass.
The direct native/CDP harness is protocol/GPU evidence, not a Chrome UI substitute.
WASM was compile-checked; this batch did not run the application in a WASM browser.

### Repeatable verification

```sh
cargo test -p rake -p rake-cli -p rake-inspector -p rake-protocol -p rake-ui -p rake-macros --features rake/view-test-support --offline
cargo clippy -p rake -p rake-cli -p rake-inspector -p rake-protocol --all-targets --features rake/view-test-support --offline -- -D warnings
cargo test -p rake --no-default-features --lib --offline
cargo check -p rake -p rake-view -p rake-inspector --lib --target wasm32-unknown-unknown --features rake/experimental-view --offline
node scripts/test-inspector.mjs
node scripts/test-inspector-ui.mjs
python3 scripts/test-inspector.py
cargo build -p rake-cli -p rake-view -p rake-routing
node scripts/test-elements-cdp.mjs
python3 scripts/test-elements.py
```

The last two commands require a macOS GUI session and fail explicitly when their
real prerequisites are unavailable. They launch isolated test applications and
clean up only their own processes/endpoints. The Node harness requires Node 22+.

### Remaining engine boundaries

Opaque third-party/cached descendants, final-frame occlusion/hitbox ownership,
used auto/percentage box edges, scrollbar metrics, complete platform accessibility
relationships and private listener closures still require additional GPUI hooks.
These remain unavailable, with no inferred geometry or invented source locations.
Live hover is retained; force-pseudo-state controls, arbitrary DOM/text mutation
and Rust source rewriting are outside this completed Elements batch. Full details
remain in [element inspection](element-inspection.md).
