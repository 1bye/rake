# Element inspection

The supported graphical frontend is [stock Chrome DevTools](devtools.md).
References below to Rake's own inspectors describe legacy prototypes, not supported
UI workflows. Current Chrome verification is recorded in [DevTools status](devtools-next.md).

The Elements panel, CLI and Rust client read the same bounded, owned snapshots.
They never hold UI entities, read source files, or evaluate arbitrary Rust/JavaScript.
Both routed and standalone `App::root` runners attach capture when configured with
`ControlConfig::FromEnvironment` and enabled by `RAKE_DEV_CONTROL=1` on native
or valid authenticated development metadata on Web. Disabled and headless hosts
report `unavailable`. An attached host reports `waiting` until its first prepaint.
A standalone host captures the application-owned root and its instrumented
children, without creating a router, tabs, entries or an operation dispatcher.
The same picker, highlight, focus, scroll and source actions are available when
registered by the captured elements. Closing the root cancels its driver, leases
and endpoint; old clients receive `RuntimeShutdown` or disconnect.

`rake::view!` automatically instruments its rendered nodes, component/entity mounts,
and ordinary text/expression children. String values display as `Text`; other
expression children display their shortened GPUI element type, including unwrapped
`Stateful`/`ViewElement`/`Inspected` names. Type-erased children display as
`AnyElement`, since GPUI does not expose their concrete type name. Inline paragraph
runs share their paragraph's boundary and carry style source metadata there. Existing manual root `.inspect(...)`
calls rename automatic built-in boundaries without adding duplicates. For GPUI
builders outside `view!`, add boundaries after configuring a styled element:

```rust,no_run
use rake::gpui::{div, prelude::*};
use rake::prelude::*;

let button = div()
    .id("save-button")
    .p_3()
    .child("Save")
    .inspect("document.save");
```

For custom elements/views without `Styled`, use
`rake::inspection::inspect(element).semantic_id("document.preview")`; styles are
then unavailable. Wrapping delegates layout, painting, GPUI state IDs and
accessibility to the original element. The semantic ID is diagnostic metadata;
it does not change the element's GPUI ID. Duplicates are allowed.

The runner instruments the workspace, tab strip and presented route outlet.
Descendants inherit the outlet's actual tab/entry ownership. Hidden tabs and
retained history entries remain logical router state and do not appear as
presented content. Source locations identify the authored macro node or manual
`.inspect` call, including in release builds; source paths can be compiler-remapped.

```sh
target/debug/rake ctl --instance APPLICATION_ID inspect tree --json
# Use an exact ID returned by that capture:
target/debug/rake ctl --instance APPLICATION_ID inspect element element-1-0 --json
```

These map to `Query::InspectTree {}` / `Reply::Elements(Snapshot)` and
`Query::InspectElement { id }` / `Reply::Element(Element)`. Errors use
`ControlError::Inspection(InspectionError)`. `info.element_inspection` reports
availability, and `schema` advertises the query tags. Both the native Unix
transport and Web development broker forward these exact protocol-1 envelopes.

A capture records the router revision at render time and its own monotonic string
identity. Each node has an opaque `element-CAPTURE-INDEX` ID, its nearest inspected
ancestor, Rust element type, semantic identity, construction site, real GPUI layout
and authored base style refinements. IDs belong to one runtime and one capture;
a changed capture expires them, even when semantic IDs repeat. Identical boundary
metadata/geometry keeps its generation across highlight-only redraws. Style-only
updates also keep the generation and element IDs, so selection survives hover changes. A restarted runtime rejects
the old request envelope. `NotRetained` means refresh the tree and explicitly
select a new node. Capture IDs are strings to avoid JavaScript numeric precision
loss. The panel keeps selected details frozen when their capture is replaced;
it never silently selects a new element with the same semantic ID.

`stable_id` is a separate optional presentation identity used by Chrome DevTools.
It follows a boundary across consecutive captures, including geometry/style
changes and keyed sibling reorders. It is scoped by the mounted window, inspected
parent, Rust type, construction site, route owner, and GPUI state identity when
available. Semantic labels and traversal indexes are not matching keys. Repeated
boundaries from the same construction site need distinct GPUI `.id(...)` values;
ambiguous siblings and their descendants omit `stable_id`. Removal, omission by
capture limits, window detach, or a changed parent/route owner ends that identity.
An element returning after an absent capture receives a new identity. Older
runtimes omit this field. It never replaces the exact capture ID or generation
required by control requests. Leased highlights follow a retained presentation
identity to its updated bounds without accepting stale control requests.

Chrome maps these identities to stable CDP node/backend IDs and sends incremental
child insertion/removal, attribute, omitted-count, and collapsed child-count events.
Unchanged nodes and expanded branches remain attached across captures. Reordered
edges use remove/insert events; delivered expansion is included when reinserting
a moved subtree. Chrome may recreate the moved subtree's frontend objects.
Runtime replacement, availability changes, legacy captures without identities,
or an oversized outbound mutation batch trigger a full document refresh.
Rake's own inspector continues to freeze replaced exact-capture selections.

Tree and overlay labels use short type names such as `Div` and `TabBar`, omitting
module paths, generic arguments and GPUI's `Stateful`/`ViewElement` wrappers.
The captured `kind` and Rake's element details retain the complete Rust type.
Chrome's Elements panel still supplies its own tag-style punctuation.

GPUI's pinned public API cannot enumerate every element/view. **Coverage is Rake
inspection boundaries**, including automatic `view!` nodes, with uninstrumented
intermediates omitted. Opaque children
and reused cached GPUI subtrees are not enumerated; their missing node count is
unknown. Layout uses window-local logical pixels after layout/offsets, before
clipping; `content_mask` separately records its intersection with the inherited
content mask and viewport for picking. `styles` contains the serialized authored
base refinement. `resolved` separately contains `properties` (GPUI's live resolved
visual style) and `hovered` (the actual hitbox state). `Div`, `Stateful<Div>` and
macro-authored button boundaries
sample GPUI's public resolver during paint, including current hover, focus, active
and group states plus inherited text from uninstrumented parents. Defaults are
included, rem lengths are normalized to pixels, and width/height are measured.
Other relative layout lengths retain their native units; this is not a browser
CSS cascade. Custom/opaque boundaries without a resolver omit `resolved`.
Older runtimes also omit it. Text boundaries expose rendered content as `text`, including
empty strings; older runtimes omit it. Content is bounded to a 16 KiB UTF-8 prefix
with `metadata_truncated` marking clipping. Editable control values are excluded.
Chrome DevTools receives native text nodes and live character-data updates.
Bounded accessibility labels can include control names. Every snapshot
carries this coverage description, even when it has no nodes.

`box_model` separates measured `border` bounds from `padding`, `content`, and
`margin` boxes, all in unclipped window-local logical pixels. The latter are derived
from the layout-phase style, using GPUI's device-pixel rounding (including its
minimum nonzero border stroke). Absolute px/rem edges are supported. Used percentage
padding, auto/percentage margins and scrollbar metrics are private to GPUI; affected
boxes are `null` with reasons in `unavailable`. Opaque elements and older runtimes
omit the model. The visible intersection remains in `content_mask`.

Resolved `layers` partition properties by observed origin: `default`, `inherited`
text context, `authored` base, `interaction` changes from base, and measured `layout`
size. This also works through uninspected text ancestors without attributing values
to the nearest captured parent. GPUI does not expose the individual interaction
rule, equal-valued overrides, or opaque per-method declaration source locations. The existing `source`
field remains the inspection boundary construction site. Older runtimes omit layers.

Automatic nodes add `view: { tag, style_sources }`. A style source records its
`scope` tag, attribute name, bounded Rust expression spelling, and attribute
`SourceLocation`. Both Rake frontends show these in Source; Chrome node properties
expose the read-only `rakeStyleSources` JSON string. They list authored declarations,
including inline paragraph declarations and untaken conditional branches; they do
not claim that a source won the live style resolution. Opaque style closures have
an attribute location, rather than fabricated per-method provenance. Style source
metadata is bounded to 4 KiB per boundary (512 bytes per expression); truncation
sets `metadata_truncated`. Older/manual boundaries omit `view`.

Generated macro site IDs and explicit loop keys preserve presentation identities
without changing underlying GPUI state IDs. Component keys accept GPUI `ElementId`,
strings, common integer keys, common named-integer tuples and UUIDs; arbitrary typed `id`
properties remain accepted but unknown types provide no inspection key. Duplicate
unkeyed sites remain explicitly ambiguous. Use `ElementId` for custom component
keys when identity must survive reordering.

Each resolved style, including provenance layers, is bounded to 16 KiB and the remaining total snapshot budget.
If it does not fit, `resolved` is omitted and `metadata_truncated` is true; its
boundary and authored metadata are retained. Both Rake frontends provide a
**Computed** tab and refresh selected details while their exact ID remains live.
The **Styles** tab retains the authored base refinements.

Capture retains at most 8192 nodes within a 32 MiB serialized metadata budget.
Paged responses stay below 512 KiB, within the transport's 1 MiB frame. Omitted inspected nodes are counted. Semantic
IDs/type names are limited to 256 UTF-8 bytes, source paths to 512, and authored style data
to 4 KiB per node. Oversized styles are unavailable; shortened/omitted metadata
sets `metadata_truncated`. Non-finite geometry is unavailable. A snapshot replaces
the previous one; there is no unbounded frame history. Source paths, semantic IDs
and supplied style values are developer metadata and should not contain secrets.

Verify portable contracts and the panel with:

```sh
cargo test -p rake --no-default-features --test element_inspection
node scripts/test-inspector-ui.mjs
```

The explicit display integration requires a macOS GUI session:

```sh
cargo build -p rake-routing -p rake-cli
python3 scripts/test-elements.py
```

It starts the real routing application, verifies measured bounds/style/source
lines and route ownership through the CLI, replaces the entry, rejects stale
IDs, and forwards the query through the real inspector HTTP proxy. It cleans up
its application, server and private control directory. It fails if display/capture
prerequisites are absent; in-memory tests do not stand in for GPU validation.

## Development highlight and picking

Mounted opt-in hosts advertise `info.element_interaction: true`; headless and
control-disabled hosts report false. `HighlightElement { id, generation }` draws
a translucent fill, one-pixel border and a kind/semantic-ID/dimensions label in
that element's window. `id: null` clears highlights. An exact generation mismatch
returns typed `StaleGeneration`; it never retargets by semantic ID. Highlights
expire after two seconds unless renewed and clear when their capture changes.

`InspectMode { enabled, lease_ms }` intercepts window mouse movement, button and
wheel events before application handlers. Escape cancels before application
shortcuts. A click picks the last/deepest instrumented boundary containing the
pointer inside both its captured bounds and content mask, then exits mode. The
matching release is swallowed too. Leases must be 1–2000 ms; input handlers check
them even without another render, and a host-owned timer repaints expired overlays.
The CDP and inspector clients renew one-second leases about every 100 ms.

`InspectPick {}` returns `mode`, `generation`, `hovered`, `picked` and
`pick_sequence`. Clicks increment the sequence once. Results remain readable so
one polling client cannot consume another's pick; each client deduplicates by
sequence. New inspect mode clears the previous selection. State is window-local
for highlights and capture geometry; inspect mode is shared across the runtime's
windows. The query selects the capture of the most recently hovered/picked window.

These are transient development commands: they return acknowledgement, retain no
operation or retry-key entry, and never modify router/application state. Closing
CDP clears its controls immediately; crashed or disconnected clients lose input
capture within one lease. Both Rake inspector frontends expose **Pick** and
hover-to-highlight controls in Elements.

```sh
rake highlight element-3-21 --session /tmp/rake-dev-EXAMPLE
rake highlight none --session /tmp/rake-dev-EXAMPLE
rake ctl --session /tmp/rake-dev-EXAMPLE inspect mode on --lease-ms 1000
rake inspect pick --session /tmp/rake-dev-EXAMPLE --json
```

The CLI obtains a generation from the current tree unless `--generation` is
explicitly supplied. Repeating `inspect pick` shows the same sequence and does
not replay a click. Enabling mode once from the CLI expires unless renewed.

Known limits: picking follows observed boundary paint order, with traversal order for legacy captures. Opaque/cached or deferred surfaces outside the host capture may
be picked incorrectly. Only instrumented boundaries can be picked, with the same
8192-node cap. Opaque descendants resolve to an inspected ancestor. Polling adds
roughly 100–250 ms of hover/selection lag and can be slower through the Web broker;
a push channel is not implemented. A new geometry/metadata capture invalidates
old IDs. No headers, bodies, source contents or uninstrumented text are captured.

## Elements navigation

Both Rake inspectors search captured names and semantic/opaque IDs. Chrome's
Elements Find (`Cmd/Ctrl+F`) supports case-insensitive name/ID substrings and an
exact semantic ID with `#id`. Results include collapsed descendants and reveal
their ancestor path without replacing known nodes. Searches are bounded to eight
retained result sets; removed nodes and runtime replacement invalidate results.
XPath and arbitrary browser selectors are not evaluated.

**Scroll into view** and **Focus element** use `NavigateElement { id, generation,
action }`, through the same authenticated protocol on native and Web. These
development controls have no retained application operation or retry key. A stale
generation never retargets. Focus applies the boundary's real handle on the next
frame and activates its native window; it does not click the element.

GPUI's pinned public API does not expose existing Div focus/scroll handles. Register
the same handles already used by the app after adding an inspection boundary:

```rust,no_run
use rake::gpui::{self, prelude::*};
use rake::inspection::InspectElement;
# fn panel(focus: &gpui::FocusHandle, scroll: &gpui::ScrollHandle) -> impl IntoElement {
gpui::div().id("viewport").overflow_y_scroll().track_scroll(scroll)
    .child(gpui::div().id("save").track_focus(focus)
        .inspect("document.save").focus_handle(focus))
    .inspect("document.viewport").scroll_handle(scroll)
# }
```

Registered scroll ancestors reveal a target from the innermost viewport outward,
with clamped offsets and minimal movement. Uninstrumented intermediate elements
are fine, but unregistered scroll containers cannot be controlled. Already fully
visible boundaries can acknowledge scrolling without a handle. `navigation`
metadata advertises support per boundary; unsupported focus or reveal returns
`NavigationUnavailable`, and Rake's buttons are disabled. Virtualized items absent
from the bounded capture cannot be found or mounted through navigation.
`navigation.focused` reports whether the registered handle currently owns keyboard focus.

```sh
rake ctl inspect focus element-3-21
rake ctl inspect scroll element-3-21
rake ctl inspect source element-3-21
```

The friendly CLI resolves the current generation for focus/scroll unless supplied
explicitly. Source opening is a separate host action: `rake.openSource($0)` for
Chrome's selected element, or `rake.openSource("document.save")` in its Console
(also accepts a retained numeric CDP node ID). Optional style/handler targets use
`rake.openSource($0, {"kind":"style","index":0})` or `kind: "handler"`.
Chrome's existing Elements context
menu supports focus/scroll through narrowly recognized native navigation helpers;
arbitrary `Runtime.callFunctionOn` JavaScript remains rejected. Chrome has no
standard external Rust-editor action; the Console helper does not add a custom
context-menu item or fork the frontend.

Source opening queries the exact capture's construction site and invokes the
system file association (`open` on macOS, `xdg-open` on Linux), without a shell.
Only existing `.rs` files whose canonical paths stay inside `RAKE_SOURCE_ROOT`
(default: the CLI/inspector host's current directory) are admitted. Set that variable
on the host when launching outside the source workspace. `rake dev` supplies the
Cargo workspace root unless the variable was set explicitly. Client requests supply
element IDs, never paths or commands. No source contents are served or captured.
The system default editor may open only the file; line/column are returned as
metadata because file associations have no portable location-jump convention.
Compiler-remapped or missing source paths fail explicitly. Standalone Python hosts
must set `RAKE_CLI` to the built CLI; `rake dev`/`rake inspect` configure it already.

For Web inspection, keep the application in a visible tab/window while using
DevTools (separate windows worked in Chrome 156). A hidden or suspended application
tab cannot service broker requests or renew inspection leases reliably. Suspension
expires the picker; it does not keep application input captured indefinitely.

## Larger trees and lazy metadata

`inspect_tree_page` returns a lightweight index in pages of at most 256 nodes.
The first request uses `generation: null, offset: 0`; subsequent requests pin the
returned capture and follow `next`. Four captures may be pinned for five seconds,
so redraws cannot splice generations. Expired pages fail explicitly. Index entries
omit styles, box models and style expressions; `inspect_element` loads those on
selection. Each response stays below 512 KiB. The legacy `inspect_tree` remains a
bounded prefix and counts the undisplayed remainder as omitted. Chrome and both
Rake inspectors use the paged index when advertised; Chrome delivers branches to
its frontend lazily. Stable presentation identities preserve selection across
captures, but removed or ambiguous nodes are never retargeted by name.

## Geometry hook requirements

Picking uses observed paint order for captured boundaries and the paint-time mask
intersected with the window viewport. Unpainted boundaries are excluded when paint
order is available. Highlight geometry uses the same clip as the target.

Complete geometry needs GPUI to expose the used layout result for a LayoutId:
resolved margin, padding, border and scrollbar insets, together with its containing
block and device rounding. Inferring these from requested CSS-like values is unsafe.
Complete picking additionally needs a final-frame ordered hitbox/occlusion stream,
including deferred draws and reused cached subtrees, with stable element ownership.
The current pinned GPUI public API supplies neither complete hook. These unknown
boxes and opaque descendants remain explicitly unavailable, rather than guessed.

## Temporary style editing

Chrome exposes an editable **DevTools base overrides** rule and declaration toggles, with
`CSS.setStyleTexts`, `CSS.setStyleSheetText` and
`CSS.setEffectivePropertyValueForNode`; `Rake.resetStyles`/`Rake.undoStyles` take a
nodeId. The exact Console helpers `rake.resetStyles($0)` and `rake.undoStyles($0)`
operate on the selected element. Authored rules and Rust source remain read-only. Chrome accepts one sheet
edit per request, rejecting unsupported batches before any mutation.

Overrides refine the real GPUI base before layout and paint. Private interaction
rules may override them; Computed always reflects the actual result. Supported
values include hex/transparent colors, px/rem dimensions and spacing, auto dimensions,
opacity, font size/weight and a literal font family. Layout edits also support display
(block/flex/grid/none), visibility, relative/absolute positioning and px/rem/auto insets,
flex direction/wrap/basis/grow/shrink, alignment, gap, and visible/clip/hidden/scroll overflow.
Borders support solid/dashed styles, colors, per-edge or uniform widths, and per-corner
or uniform radii. Grid templates use `repeat(N, minmax(0|min-content|max-content, 1fr))`
with 1–256 equal tracks, matching GPUI’s public grid representation. Unsupported CSS
(e.g. arbitrary tracks, sticky positioning and overflow:auto) is rejected. Unsupported declarations,
non-finite values, duplicates and oversized lists fail atomically. There are at
most 32 declarations and 32 undo steps per edited boundary, across 64 boundaries.
Edits follow an unambiguous stable presentation identity and expire on removal or
runtime replacement. Stale capture commands never select another element. Delivery
is transient and never automatically retried, including undo.

## Accessibility and interaction metadata

Both Rake Elements frontends expose **Accessibility** and **Events** tabs. Chrome
supports Accessibility root/full/partial/child/ancestor queries and node updates;
its Properties pane also exposes read-only `rakeAccessibility` and `rakeHandlers`
JSON strings. This projects inspected GPUI boundaries, not the complete platform
accessibility tree. Opaque nodes and synthetic accessibility descendants are not
enumerated. Full-tree responses are bounded; use root/child queries for larger trees.

Roles, labels, descriptions, disabled/hidden state and supported actions come from
the element's real accessibility writer. Labels include macro-generated control
names, so accessible names may contain visible button text. Labels/descriptions are
bounded to 256/512 UTF-8 bytes. Editable values, passwords, listener closures and captured state are not retained.
Rendered text is captured separately. Accessible `name` prioritizes explicit labels
(including explicit empty labels), then a text boundary’s own content, then captured
visible descendant text for controls supporting names from content. `name_source`
reports `label`, `text`, or `contents`. Names are UTF-8 prefixes bounded to 256 bytes.
Hidden branches are excluded. Opaque descendants, relationships and synthetic AX
children prevent claiming the complete platform accessible-name algorithm.

Captured states include selected/expanded when applicable, checked/pressed tri-state,
required, readonly, busy, multiselectable and invalid. Chrome maps text roles to
StaticText and sends name/state changes without replacing the corresponding DOM node.
Unknown names/focus facts remain unavailable.

Registered focus handles report actual focus, containment, tab index and tab-stop
state. Macro buttons register their real internal handles automatically. Captured
order sorts known tab stops by GPUI tab index then observed paint order; private
focus groups and opaque tab stops prevent claiming complete application focus order.

Macro-authored `on:*` bindings include event/action names and attribute locations.
Manual builders can call `.inspect_handler("click")` after registering a listener;
this records metadata only and never installs or invokes a callback. At most 32
handler sites / 8 KiB per boundary are retained, with explicit metadata truncation.
The registration site does not locate the body of an opaque callback. Chrome's
JavaScript Event Listeners/Debugger APIs are not supplied for Rust closures.

### Captured Rust source targets

Source and Events panes open individual authored style attributes and handler
registration sites using the system default editor. These are captured sites, not
locations inferred inside arbitrary closures. The host resolves the exact element
and a typed target, then applies the same workspace-confined Rust-file validation.
It never accepts a client-supplied path.

CLI: `rake ctl inspect source ELEMENT_ID --style 0` or `--handler 0`.
CDP: `Rake.openSource { nodeId, target: { kind: "style", index: 0 } }`, or
Console: `rake.openSource({"nodeId":42,"target":{"kind":"handler","index":0}})`.
Indices refer to captured `view.style_sources` and `handlers`; unavailable indices
fail explicitly. Omit the target to open the construction site as before. The
system default opener may ignore line/column; those remain in the receipt.

### Tree and copy actions

Chrome supplies its normal disclosure, keyboard navigation, recursive expansion,
copy and context menus. Search by name, semantic ID or text reveals collapsed
ancestors without rebuilding known nodes. Chrome's Copy element/outerHTML uses
`DOM.getOuterHTML`; Copy selector is generated by Chrome from the synthetic tree.

Copies contain retained metadata only; clipped text and opaque children cannot be
reconstructed. Text includes hidden captured descendants, like textContent. Markup
is the synthetic inspection tree, not Rust source. Paths describe root-relative
node names, sibling ordinals and quoted semantic IDs; they are not executable CSS
or XPath selectors. Invalid/cyclic graphs and oversized copies fail explicitly
instead of returning partial output. Markup also bounds JSON-escaped wire size.

The picker excludes captured hidden branches, including descendants without their
own style reader. Disabled visible controls remain inspectable. The HTTP inspector
keeps its tree action toolbar visible while scrolling and defers tree replacement
through click dispatch; unrelated diagnostic traffic does not rebuild the tree.
