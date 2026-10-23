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
recapture expires them, even when semantic IDs repeat. A restarted runtime rejects
the old request envelope. `NotRetained` means refresh the tree and explicitly
select a new node. Capture IDs are strings to avoid JavaScript numeric precision
loss. The panel keeps selected details frozen when their capture is replaced;
it never silently selects a new element with the same semantic ID.

GPUI's pinned public API cannot enumerate every element/view. **Coverage is Rake
inspection boundaries**, with uninstrumented intermediates omitted. Opaque children
and reused cached GPUI subtrees are not enumerated; their missing node count is
unknown. Layout uses window-local logical pixels after layout/offsets, before
clipping; it is not a screenshot or a visibility/hit-testing claim. Styles are the
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
