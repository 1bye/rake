# Element inspection

The graphical frontend is [Chrome DevTools](devtools.md). References below to Rake's own
inspectors describe [legacy prototypes](internal/legacy-inspectors.md).

The Elements panel, CLI and Rust client read the same bounded, owned snapshots. They never hold UI entities, read source files, or evaluate arbitrary Rust or JavaScript.

**Enabling it.** Capture and temporary editing need the opt-in `rake/devtools` feature. Without it, instrumentation and the backend are compiled out, manual builders return their input, and queries report unavailable. Use `inspect_with` for lazy dynamic identities. See [build configuration](devtools.md#connection).

Routed and standalone `App::root` and `App::root_with` runners attach capture when configured with `ControlConfig::FromEnvironment` and enabled by `RAKE_DEV_CONTROL=1` on native, or valid authenticated development metadata on Web. Disabled and headless hosts report `unavailable`, and an attached host reports `waiting` until its first prepaint.

**Standalone hosts** capture the app-owned root and its instrumented children without creating a router, tabs or entries. Their managed runtime still provides command, work, quit and update operations and inspection control, and the same picker, highlight, focus, scroll and source actions work when registered by captured elements.

With the default `ClosePolicy::Quit`, closing the root stops managed work and cancels its driver, leases and endpoint. `ClosePolicy::KeepRunning` may leave the runtime alive after the window detaches. Old clients get `RuntimeShutdown` or disconnect once the runtime ends.

`rake::view!` automatically instruments its rendered nodes, component and entity mounts, and ordinary text and expression children.

- String values display as `Text`.
- Other expression children display their shortened GPUI element type, including unwrapped `Stateful`, `ViewElement` and `Inspected` names.
- Type-erased children display as `AnyElement`, because GPUI doesn't expose their concrete type.
- Inline paragraph runs share their paragraph's boundary and carry style metadata there.
- Manual root `.inspect(...)` calls rename automatic built-in boundaries without adding duplicates.

For GPUI builders outside `view!`, add boundaries after configuring a styled element:

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

A capture records the router revision at render time and its own monotonic string identity. Each node has an opaque `element-CAPTURE-INDEX` ID, its nearest inspected ancestor, Rust element type, semantic identity, construction site, real GPUI layout and authored base style refinements.

- IDs belong to one runtime and one capture. A changed capture expires them, even when semantic IDs repeat. They're strings to avoid JavaScript precision loss.
- Identical metadata and geometry keep their generation across highlight-only redraws. Style-only updates also keep the generation and IDs, so selection survives hover changes.
- A restarted runtime rejects the old request envelope.
- `NotRetained` means refresh the tree and select a new node.

The panel keeps selected details frozen when their capture is replaced, and never silently selects a new element with the same semantic ID.

`stable_id` is a separate optional identity used by Chrome DevTools. It follows a boundary across consecutive captures, including geometry and style changes and keyed sibling reorders. It never replaces the exact capture ID or generation that control requests need.

- It's scoped by mounted window, inspected parent, Rust type, construction site, route owner and GPUI state identity when available. Semantic labels and traversal indexes aren't matching keys.
- Repeated boundaries from one site need distinct GPUI `.id(...)` values. Ambiguous siblings and their descendants omit `stable_id`.
- Removal, omission by capture limits, window detach or a changed parent or route owner ends it. An element returning after an absent capture gets a new identity.
- Older runtimes omit the field.

Leased highlights follow a retained identity to its updated bounds without accepting stale control requests.

Chrome maps these identities to stable CDP node and backend IDs, and sends incremental child insertion and removal, attribute, omitted-count and collapsed child-count events. Unchanged nodes and expanded branches stay attached across captures.

Reordered edges use remove and insert events, including delivered expansion, though Chrome may recreate the moved subtree's frontend objects. Runtime replacement, availability changes, legacy captures without identities or an oversized mutation batch trigger a full refresh. Rake's own inspector still freezes replaced exact-capture selections.

Tree and overlay labels use short type names such as `Div` and `TabBar`, omitting
module paths, generic arguments and GPUI's `Stateful`/`ViewElement` wrappers.
The captured `kind` and Rake's element details retain the complete Rust type.
Chrome's Elements panel still supplies its own tag-style punctuation.

GPUI's public API can't enumerate every element or view. **Coverage is Rake inspection boundaries**, including automatic `view!` nodes, with uninstrumented intermediates omitted. Opaque children and reused cached subtrees aren't enumerated, and their missing node count is unknown. Every snapshot carries this coverage description, even with no nodes.

**Layout** uses window-local logical pixels after layout and offsets, before clipping. `content_mask` separately records its intersection with the inherited mask and viewport, for picking.

**Styles**

- `styles` holds the serialized authored base refinement.
- `resolved` holds `properties` (GPUI's live resolved visual style) and `hovered` (the actual hitbox state).
- `Div`, `Stateful<Div>` and macro-authored buttons sample GPUI's public resolver during paint, including hover, focus, active and group states, plus inherited text from uninstrumented parents.
- Defaults are included, rem lengths are normalized to pixels, and width and height are measured. Other relative lengths keep their native units, so this isn't a browser cascade.
- Custom or opaque boundaries without a resolver, and older runtimes, omit `resolved`.

**Text**

- Text boundaries expose rendered content as `text`, including empty strings. Older runtimes omit it.
- Content is bounded to a 16 KiB UTF-8 prefix, with `metadata_truncated` marking clipping.
- Editable control values are excluded.
- Chrome receives native text nodes and live character-data updates.
- Bounded accessibility labels can include control names.

`box_model` separates measured `border` bounds from `padding`, `content` and `margin` boxes, all in unclipped window-local logical pixels. The latter come from the layout-phase style, using GPUI's device-pixel rounding, including its minimum nonzero border stroke.

Absolute px and rem edges are supported. Used percentage padding, auto or percentage margins and scrollbar metrics are private to GPUI, so affected boxes are `null` with reasons in `unavailable`. Opaque elements and older runtimes omit the model. The visible intersection stays in `content_mask`.

Resolved `layers` partition properties by observed origin: `default`, `inherited` text context, `authored` base, `interaction` changes from base, and measured `layout` size. This works through uninspected text ancestors without attributing values to the nearest captured parent.

GPUI doesn't expose the individual interaction rule, equal-valued overrides or per-method declaration locations. `source` stays the boundary's construction site, and older runtimes omit layers.

Automatic nodes add `view: { tag, style_sources }`. A style source records its `scope` tag, attribute name, bounded Rust expression spelling and attribute `SourceLocation`. Chrome exposes these in the read-only `rakeStyleSources` JSON string.

They list authored declarations, including inline paragraph declarations and untaken conditional branches, and don't claim which one won. Opaque style closures have an attribute location, not fabricated per-method provenance. Metadata is bounded to 4 KiB per boundary and 512 bytes per expression, with `metadata_truncated` on truncation. Older and manual boundaries omit `view`.

Generated macro site IDs and explicit loop keys keep presentation identities without changing GPUI state IDs. Component keys accept `ElementId`, strings, common integer keys, common named-integer tuples and UUIDs. Other typed `id` properties are accepted but provide no inspection key, and duplicate unkeyed sites stay ambiguous. Use `ElementId` for custom component keys when identity must survive reordering.

Each resolved style, including layers, is bounded to 16 KiB and the remaining snapshot budget. If it doesn't fit, `resolved` is omitted and `metadata_truncated` is true, while the boundary and authored metadata stay. Chrome’s **Computed** tab refreshes while the exact ID is live, and **Styles** keeps the authored refinements.

| Limit | Value |
| --- | --- |
| Nodes | 8192, within a 32 MiB serialized metadata budget. Omitted nodes are counted |
| Paged responses | Below 512 KiB, within the 1 MiB transport frame |
| Semantic IDs and type names | 256 UTF-8 bytes |
| Source paths | 512 bytes |
| Authored style data | 4 KiB per node. Oversized styles are unavailable |

Shortened or omitted metadata sets `metadata_truncated`, and non-finite geometry is unavailable. A snapshot replaces the previous one, so there's no unbounded history.

> Source paths, semantic IDs and supplied style values are developer metadata, and shouldn't contain secrets.

Verify the portable inspection contracts with:

```sh
cargo test -p rake --no-default-features --test element_inspection
```

The explicit display integration requires a macOS GUI session:

```sh
cargo build -p rake-routing -p rake-cli --features rake/devtools
python3 scripts/test-elements.py
```

It starts the real routing app and cleans up its app, server and private control directory. It fails if display or capture prerequisites are absent, because in-memory tests don't stand in for GPU validation.

## Development highlight and picking

Mounted opt-in hosts advertise `info.element_interaction: true`, and headless and control-disabled hosts report false.

`HighlightElement { id, generation }` draws a translucent fill, one-pixel border and a kind, semantic ID and dimensions label in that element's window. `id: null` clears highlights. A generation mismatch returns typed `StaleGeneration` and never retargets by semantic ID. Highlights expire after two seconds unless renewed, and clear when their capture changes.

`InspectMode { enabled, lease_ms }` intercepts window mouse movement, button and wheel events before application handlers, and Escape cancels before app shortcuts.

A click picks the deepest instrumented boundary containing the pointer inside both its captured bounds and content mask, then exits the mode. The matching release is swallowed too.

Leases must be 1–2000 ms. Input handlers check them even without another render, and a host-owned timer repaints expired overlays. The CDP and inspector clients renew one-second leases about every 100 ms.

`InspectPick {}` returns `mode`, `generation`, `hovered`, `picked` and `pick_sequence`. Clicks increment the sequence once. Results stay readable, so one polling client can't consume another's pick, and each client deduplicates by sequence. A new inspect mode clears the previous selection.

Highlights and capture geometry are window-local, while inspect mode is shared across the runtime's windows. The query selects the capture of the most recently hovered or picked window.

These are transient development commands: they return acknowledgement, retain no
operation or retry-key entry, and never modify router/application state. Closing
CDP clears its controls immediately; crashed or disconnected clients lose input
capture within one lease. Chrome DevTools exposes picking and hover highlights
in Elements.

```sh
rake highlight element-3-21 --session /tmp/rake-dev-EXAMPLE
rake highlight none --session /tmp/rake-dev-EXAMPLE
rake ctl --session /tmp/rake-dev-EXAMPLE inspect mode on --lease-ms 1000
rake inspect pick --session /tmp/rake-dev-EXAMPLE --json
```

The CLI obtains a generation from the current tree unless `--generation` is
explicitly supplied. Repeating `inspect pick` shows the same sequence and does
not replay a click. Enabling mode once from the CLI expires unless renewed.

**Known limits**

- Picking follows observed boundary paint order, with traversal order for legacy captures.
- Opaque, cached or deferred surfaces outside the host capture may be picked incorrectly.
- Only instrumented boundaries can be picked, with the same 8192-node cap. Opaque descendants resolve to an inspected ancestor.
- Polling adds roughly 100–250 ms of hover and selection lag, and can be slower through the Web broker. There's no push channel.
- A new geometry or metadata capture invalidates old IDs.
- No headers, bodies, source contents or uninstrumented text are captured.

## Elements navigation

Chrome's Elements Find (`Cmd/Ctrl+F`) supports case-insensitive name and ID substrings, and an exact semantic ID with `#id`. Results include collapsed descendants and reveal their ancestor path without replacing known nodes.

Searches are bounded to eight retained result sets, and removed nodes and runtime replacement invalidate results. XPath and arbitrary browser selectors aren't evaluated.

**Scroll into view** and **Focus element** use `NavigateElement { id, generation, action }`, through the same authenticated protocol on native and Web. These development controls keep no application operation or retry key, and a stale generation never retargets. Focus applies the boundary's real handle on the next frame and activates its native window. It doesn't click the element.

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

Registered scroll ancestors reveal a target from the innermost viewport outward, with clamped offsets and minimal movement. Uninstrumented intermediates are fine, but unregistered scroll containers can't be controlled. Fully visible boundaries can acknowledge scrolling without a handle.

`navigation` metadata advertises support per boundary, and `navigation.focused` reports whether the registered handle owns keyboard focus. Unsupported focus or reveal returns `NavigationUnavailable`, and Rake's buttons are disabled. Virtualized items absent from the capture can't be found or mounted through navigation.

```sh
rake ctl inspect focus element-3-21
rake ctl inspect scroll element-3-21
rake ctl inspect source element-3-21
```

The friendly CLI resolves the current generation for focus and scroll unless you supply one.

Source opening is a separate host action:

- `rake.openSource($0)` for Chrome's selected element.
- `rake.openSource("document.save")` in Console, which also accepts a retained numeric CDP node ID.
- `rake.openSource($0, {"kind":"style","index":0})` or `kind: "handler"` for style and handler targets.

Chrome's Elements context menu supports focus and scroll through narrowly recognized native navigation helpers, and arbitrary `Runtime.callFunctionOn` JavaScript stays rejected. Chrome has no standard external Rust-editor action, and this helper doesn't add a menu item or fork the frontend.

Source opening queries the exact capture's construction site and invokes the system file association (`open` on macOS, `xdg-open` on Linux), without a shell.

- Only existing `.rs` files whose canonical paths stay inside `RAKE_SOURCE_ROOT` are admitted. It defaults to the host's current directory, so set it when launching outside the workspace. `rake dev` supplies the Cargo workspace root.
- Clients supply element IDs, never paths or commands. No source contents are served or captured.
- The default editor may open only the file. Line and column come back as metadata, because file associations have no portable jump convention.
- Compiler-remapped or missing source paths fail explicitly.
- Standalone Python hosts must set `RAKE_CLI` to the built CLI. `rake dev` configures it automatically.

For Web inspection, keep the application in a visible tab/window while using
DevTools (separate windows worked in Chrome 156). A hidden or suspended application
tab cannot service broker requests or renew inspection leases reliably. Suspension
expires the picker; it does not keep application input captured indefinitely.

## Larger trees and lazy metadata

`inspect_tree_page` returns a lightweight index in pages of at most 256 nodes, each response below 512 KiB. The first request uses `generation: null, offset: 0`, and later ones pin the returned capture and follow `next`.

- Four captures may be pinned for five seconds, so redraws can't splice generations. Expired pages fail explicitly.
- Index entries omit styles, box models and style expressions, and `inspect_element` loads those on selection.
- Legacy `inspect_tree` stays a bounded prefix, counting the rest as omitted.

Chrome uses the paged index when advertised and loads branches lazily. Stable identities keep selection across captures, but removed or ambiguous nodes are never retargeted by name.

## Capture cost and cache lifetime

A frame is published once, after paint, by moving the completed capture into the
window store. Stable-boundary lookups use a hash index. Authored/refined style JSON,
resolved properties and provenance are shared immutable payloads; lightweight tree
pages never clone that data. Full owned metadata is materialized when requested.

Each mounted window caches its last style inputs and bounded source metadata per unambiguous stable boundary. Changes to authored styles, inherited text, rem size, interaction state or temporary overrides invalidate resolved data. Current hover and measured size come from the requested frame, so moving or resizing a node doesn't rebuild unchanged style JSON.

Removal, ambiguity, capture omission and window detach discard the caches. Pinned pages keep immutable payloads only for their bounded lifetime.

Byte admission uses conservative allocation-free JSON counts, with an exact counting writer near limits, and unchanged metadata reuses its count across coordinate changes. Snapshot and transport limits still apply.

Native Unix requests decode and encode owned messages on the blocking pool, while UI dispatch stays on the local thread. The 32-connection bound and five-second deadline remain.

## Geometry hook requirements

Picking uses observed paint order for captured boundaries and the paint-time mask
intersected with the window viewport. Unpainted boundaries are excluded when paint
order is available. Highlight geometry uses the same clip as the target.

Complete geometry needs GPUI to expose the used layout result for a `LayoutId`: resolved margin, padding, border and scrollbar insets, with its containing block and device rounding. Inferring these from requested values is unsafe.

Complete picking also needs a final-frame ordered hitbox and occlusion stream, including deferred draws and reused cached subtrees, with stable element ownership. GPUI's public API supplies neither, so unknown boxes and opaque descendants stay explicitly unavailable rather than guessed.

## Temporary style editing

Chrome exposes an editable **DevTools base overrides** rule and declaration toggles, with
`CSS.setStyleTexts`, `CSS.setStyleSheetText` and
`CSS.setEffectivePropertyValueForNode`; `Rake.resetStyles`/`Rake.undoStyles` take a
nodeId. The exact Console helpers `rake.resetStyles($0)` and `rake.undoStyles($0)`
operate on the selected element. Authored rules and Rust source remain read-only. Chrome accepts one sheet
edit per request, rejecting unsupported batches before any mutation.

Overrides refine the real GPUI base before layout and paint. Private interaction rules may override them, and Computed always shows the actual result.

| Area | Supported values |
| --- | --- |
| Colors and text | Hex and transparent colors, opacity, font size and weight, a literal font family |
| Size and spacing | px and rem dimensions and spacing, auto dimensions |
| Display | block, flex, grid, none; visibility |
| Position | relative, absolute; px, rem and auto insets |
| Flex | direction, wrap, basis, grow, shrink, alignment, gap |
| Overflow | visible, clip, hidden, scroll |
| Borders | solid and dashed, colors, per-edge or uniform widths, per-corner or uniform radii |
| Grid | `repeat(N, minmax(0\|min-content\|max-content, 1fr))` with 1–256 equal tracks |

Other CSS, such as arbitrary tracks, sticky positioning and `overflow:auto`, is rejected. Unsupported declarations, non-finite values, duplicates and oversized lists fail atomically.

There are at most 32 declarations and 32 undo steps per boundary, across 64 boundaries. Edits follow an unambiguous stable identity and expire on removal or runtime replacement. Stale commands never select another element, and delivery is transient and never retried, including undo.

## Temporary text editing

Temporary text editing works on text boundaries where `text_editable` is true. `EditElementText { id, generation, edit }` accepts `TextEdit::Set { text }`, `Reset` and `Undo`.

- Exact runtime and capture IDs are required, requests have no retry keys, and stale targets never retarget.
- Overrides apply before request-layout, leaving app-owned content intact.
- Composed paragraphs use base formatting during an override and recover their inline runs on reset.
- Histories are bounded to 64 nodes, 32 undo steps, 16 KiB per replacement and 4 MiB overall, and expire with stable boundaries.

Chrome sends `DOM.setNodeValue`; see [text editing controls](devtools.md) for reset and undo helpers. Without `devtools`, the backend and override lookups are compiled out.

## Accessibility and interaction metadata

Chrome supports Accessibility root, full, partial, child and ancestor queries and node updates, and its Properties pane exposes read-only `rakeAccessibility` and `rakeHandlers` JSON strings.

This projects inspected GPUI boundaries, not the complete platform accessibility tree. Opaque nodes and synthetic descendants aren't enumerated. Full-tree responses are bounded, so use root and child queries for larger trees.

Roles, labels, descriptions, disabled and hidden state and supported actions come from the element's real accessibility writer. Labels include macro-generated control names, so they may contain visible button text, and are bounded to 256 bytes (descriptions to 512). Editable values, passwords, listener closures and captured state aren't kept, and rendered text is captured separately.

The accessible `name` is a UTF-8 prefix bounded to 256 bytes, chosen in this order, as reported by `name_source`:

1. `label`: explicit labels, including explicit empty ones.
2. `text`: a text boundary's own content.
3. `contents`: captured visible descendant text, for controls supporting names from content.

Hidden branches are excluded. Opaque descendants, relationships and synthetic children mean this isn't the complete platform accessible-name algorithm.

Captured states include selected/expanded when applicable, checked/pressed tri-state,
required, readonly, busy, multiselectable and invalid. Chrome maps text roles to
StaticText and sends name/state changes without replacing the corresponding DOM node.
Unknown names/focus facts remain unavailable.

Registered focus handles report actual focus, containment, tab index and tab-stop
state. Macro buttons register their real internal handles automatically. Captured
order sorts known tab stops by GPUI tab index then observed paint order; private
focus groups and opaque tab stops prevent claiming complete application focus order.

Macro-authored `on:*` bindings include event or action names and attribute locations. Manual builders can call `.inspect_handler("click")` after registering a listener, which records metadata only and never installs or invokes a callback.

At most 32 handler sites or 8 KiB per boundary are kept, with explicit truncation. The registration site doesn't locate the body of an opaque callback, and Chrome's JavaScript Event Listeners and Debugger APIs aren't supplied for Rust closures.

### Captured Rust source targets

The CLI and Chrome DevTools can open captured style attributes and handler
registration sites in the system default editor. The host resolves the exact element
and a typed target, then applies the same workspace-confined Rust-file validation.
It never accepts a client-supplied path.

- **CLI:** `rake ctl inspect source ELEMENT_ID --style 0` or `--handler 0`.
- **CDP:** `Rake.openSource { nodeId, target: { kind: "style", index: 0 } }`.
- **Console:** `rake.openSource({"nodeId":42,"target":{"kind":"handler","index":0}})`.

Indices refer to captured `view.style_sources` and `handlers`, and unavailable indices fail explicitly. Omit the target to open the construction site. The default opener may ignore line and column, which stay in the receipt.

### Tree and copy actions

Chrome supplies its normal disclosure, keyboard navigation, recursive expansion,
copy and context menus. Search by name, semantic ID or text reveals collapsed
ancestors without rebuilding known nodes. Chrome's Copy element/outerHTML uses
`DOM.getOuterHTML`; Copy selector is generated by Chrome from the synthetic tree.

Copies contain retained metadata only, so clipped text and opaque children can't be reconstructed.

- Text includes hidden captured descendants, like `textContent`.
- Markup is the synthetic inspection tree, not Rust source, and also bounds JSON-escaped wire size.
- Paths describe root-relative node names, sibling ordinals and quoted semantic IDs. They aren't executable CSS or XPath selectors.
- Invalid or cyclic graphs and oversized copies fail explicitly, rather than returning partial output.

The picker excludes captured hidden branches, including descendants without their
own style reader. Disabled visible controls remain inspectable.
