# Element inspection

The Elements panel, CLI and Rust client read the same bounded, owned snapshots.
They never hold UI entities, read source files, or evaluate arbitrary Rust/JavaScript.
The routed application runner attaches capture when configured with
`ControlConfig::FromEnvironment` and enabled by `RAKE_DEV_CONTROL=1` on native
or valid authenticated development metadata on Web. Disabled and headless hosts
report `unavailable`. An attached host reports `waiting` until its first prepaint.

Add boundaries after configuring a styled element:

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
presented content. Source locations identify the `.inspect` call, including in
release builds; source paths can be compiler-remapped.

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
metadata/geometry keeps its generation across highlight-only redraws. A restarted runtime rejects
the old request envelope. `NotRetained` means refresh the tree and explicitly
select a new node. Capture IDs are strings to avoid JavaScript numeric precision
loss. The panel keeps selected details frozen when their capture is replaced;
it never silently selects a new element with the same semantic ID.

Tree and overlay labels use short type names such as `Div` and `TabBar`, omitting
module paths, generic arguments and GPUI's `Stateful`/`ViewElement` wrappers.
The captured `kind` and Rake's element details retain the complete Rust type.
Chrome's Elements panel still supplies its own tag-style punctuation.

GPUI's pinned public API cannot enumerate every element/view. **Coverage is Rake
inspection boundaries**, with uninstrumented intermediates omitted. Opaque children
and reused cached GPUI subtrees are not enumerated; their missing node count is
unknown. Layout uses window-local logical pixels after layout/offsets, before
clipping; `content_mask` separately records its intersection with the inherited
content mask and viewport for picking. Styles are the
serialized authored GPUI base refinement captured at the annotation, not computed,
inherited, hover or focus styles. Text contents are excluded. Every snapshot
carries this coverage description, even when it has no nodes.

Capture retains at most 256 nodes and less than 512 KiB of serialized data, leaving
room in the transport's 1 MiB frame. Omitted inspected nodes are counted. Semantic
IDs/type names are limited to 256 UTF-8 bytes, source paths to 512, and style data
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

Known limits: z-order follows prepaint order, so deferred/absolute surfaces may
be picked incorrectly. Only instrumented boundaries can be picked, with the same
256-node cap. Opaque descendants resolve to an inspected ancestor. Polling adds
roughly 100–250 ms of hover/selection lag and can be slower through the Web broker;
a push channel is not implemented. A new geometry/metadata capture invalidates
old IDs. No headers, bodies, source contents or uninstrumented text are captured.

For Web inspection, keep the application in a visible tab/window while using
DevTools (separate windows worked in Chrome 156). A hidden or suspended application
tab cannot service broker requests or renew inspection leases reliably. Suspension
expires the picker; it does not keep application input captured indefinitely.
