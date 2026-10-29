# DevTools status

Stock Chrome DevTools through the CDP adapter is the supported frontend. The HTML
inspector and `rake-inspector` are legacy prototypes. Their previous completion
checks are historical evidence, not Chrome UI verification.

The original five implementation phases are delivered: paged trees and stable
identities; picking and geometry; live styles and temporary edits; frame recording;
and captured accessibility/handler metadata. Each has a separate commit. Coverage
is limited to captured GPUI boundaries and the engine facts exposed by GPUI.

Inspection is compiled only with the opt-in `rake/devtools` feature; `rake dev`
enables it automatically. Production build configuration is documented in
[devtools.md](devtools.md#connection).

## Elements coverage checklist

| Workflow | Supported behavior | Verification |
| --- | --- | --- |
| Tree and text | Automatic `view!` and explicit boundaries; text/inline runs; lazy expansion; stable identities; incremental updates | Stock Chrome recursive expansion and inline/rich text; live Count update preserved selection; native/CDP regression harness |
| Search/navigation | Name, semantic ID and text search; reveal collapsed ancestors; focus and scroll into view | Stock Chrome name/ID/text searches and context actions; runtime navigation tests |
| Picking/highlight | Observed paint order and clipping; hidden branches excluded; leased pointer capture | Stock Chrome picker selected Increment without activating it; GPUI input/geometry tests and native/CDP harness |
| Styles/Computed | Live resolved values and observed origin layers; temporary validated edits and declaration checkboxes | Stock Chrome width edit, disable/re-enable and measured 240px result; Computed color/typography; stock Chrome pointer enter/leave changed Computed background color; native flex/grid/border tests |
| Edit stability | Index pages retain hydrated style comparison data; only real changes invalidate styles; acknowledged declarations are immediately readable | Before/after Chrome editor interaction; paged polling regression and Chrome-shaped preview/commit/toggle ranges through live CDP |
| Source | `$0`, `rake.openSource($0)` and explicit style/handler source targets; system default editor | CDP selection/context/lifetime and source hydration tests; host path confinement tests; Console UI source opening not verified in this pass |
| Accessibility | Captured roles, names, name origins, states and focus | Stock Chrome full AX tree and Accessibility sidebar; focused Increment, disabled Reset and expanded history; switch back retained selection |
| Copy | Escaped synthetic outerHTML and Chrome-generated selectors | Stock Chrome Copy outerHTML produced `<button id="increment">Increment</button>`; Copy selector produced `#increment` |
| Reset/undo | `rake.resetStyles($0)` and `rake.undoStyles($0)` exact Console helpers; `Rake.resetStyles`/`Rake.undoStyles` CDP commands | Live native/CDP harness; Chrome Console helper UI not verified in this pass |

## Stock Chrome verification

On 2026-10-29, Google Chrome **156.0.8078.48**, using its bundled
`devtools_app.html` and the actual native `rake-view` GPUI application, was checked
through native UI automation. No custom inspector was used as a substitute.

The UI pass covered lazy/recursive expansion, inline and rich text, searches from
collapsed ancestors, context navigation, outerHTML/selector copying, style editing
and toggling, Computed values, accessibility names/states/focus and returning to
the DOM tree. A native counter update changed the Chrome text without losing the
selected button. This pass found and fixed repeated false CSS invalidations and
preview/commit reads of pre-edit declaration state.

Chrome blocked pasting into Console with its safety guard. The guard was left
enabled; this pass does not establish UI verification of Console source opening
or reset/undo helpers. Those paths are covered at the adapter/runtime boundary.
The native picker and live hover were also exercised in Chrome: picking exited
without incrementing the counter, and pointer enter/leave changed the selected
button background between a resolved hover color and transparent. WASM evidence
is compilation, not browser/GPU execution.

## Repeatable verification

```sh
cargo test -p rake-cli --offline
cargo test -p rake --features view-test-support,devtools --lib inspection --offline
cargo clippy -p rake-cli -p rake --all-targets --features rake/view-test-support --offline -- -D warnings
cargo test -p rake --no-default-features --lib --offline
cargo check -p rake -p rake-view --lib --target wasm32-unknown-unknown --features rake/experimental-view --offline
cargo build -p rake-cli -p rake-view --features rake/devtools --offline
node scripts/test-elements-cdp.mjs
```

The Node harness requires Node 22+ and a macOS GUI session, and fails explicitly
when real prerequisites are unavailable. It launches isolated applications and
cleans up only its own processes/endpoints. It verifies native GPUI and CDP behavior;
it does not automate Chrome. Repeat Chrome's UI pass separately using the launch
instructions in [devtools.md](devtools.md).

## Remaining Elements boundaries

Opaque third-party/cached descendants, final-frame occlusion/hitbox ownership,
used auto/percentage box edges, scrollbar metrics, complete platform accessibility
relationships and private listener closures require additional GPUI hooks. These
remain unavailable, with no inferred geometry or invented source locations.

Chrome's browser-only controls are not all meaningful for GPUI. Force pseudo
states, arbitrary DOM/text edits, new CSS rules/selector edits, browser DOM undo/redo,
persistent flex/grid overlays, browser animation/font diagnostics, general
JavaScript evaluation and Rust source rewriting remain unsupported. Chrome's
Sources/debugger is not a Rust debugger. The Console helper UI checks noted above remain verification work, rather than
a claim of full parity.
See [element inspection](element-inspection.md) for the engine coverage contract.
