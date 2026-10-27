# Experimental scrolling and overlays

Enable Rake's `experimental-view` feature. Scrolling and virtualization use GPUI's
existing elements; overlay controls live in `rake::overlay` and use ordinary typed
component tags. There is no additional `view!` grammar or component cache.

## Scrolling and large lists

Retain `ScrollHandle` in the owning view for a normal scroll area:

```rust
# use rake::gpui::{self, IntoElement, ScrollHandle};
# fn content(scroll: &ScrollHandle) -> impl IntoElement {
rake::view! {
    <section scroll={&scroll} style={|s| s.h(gpui::px(240.)).overflow_y_scroll()}>
        <p>"Scrollable content"</p>
    </section>
}
# }
```

For thousands of equal-height rows, store a `UniformListScrollHandle` and embed
`gpui::uniform_list`. Its callback receives only the required range, including
GPUI's measurement/overscan rows. Give the viewport a bounded height and every
row the same height. Keys come from stable model IDs; row-local state belongs in
the model or retained entities because offscreen elements unmount.

```rust
# use rake::gpui::{self, prelude::*};
# fn rows(scroll: &gpui::UniformListScrollHandle) -> impl IntoElement {
rake::view! {
    <section style={|s| s.h(gpui::px(240.)).overflow_hidden()}>
        {gpui::uniform_list("records", 10_000, |range: std::ops::Range<usize>, _, _| {
            range.map(|id| rake::view! {
                <div id={("record", id)} style={|s| s.h(gpui::px(32.))}>
                    {format!("Record {id}")}
                </div>
            }).collect::<Vec<_>>()
        }).track_scroll(scroll).h(gpui::px(240.))}
    </section>
}
# }
```

Call `scroll.scroll_to_item(index, gpui::ScrollStrategy::Nearest)` and notify the
owner to jump. For variable-height content, embed GPUI's `list` with a retained
`ListState`; update that state with the model's insertions/deletions. The macro
adds no reconciliation for those APIs.

## Overlay ownership and composition

Create one host in the window's owning entity, outside `render`, then create
anchors and overlay handles from it:

```rust
# use rake::gpui::Context;
# use rake::overlay::OverlayHostState;
# fn create<T: 'static>(cx: &Context<T>) {
let overlays = OverlayHostState::new(cx);
let actions_anchor = overlays.anchor();
let actions = overlays.popover(cx);
let documents = overlays.dialog(cx);
# let _ = (actions_anchor, actions, documents);
# }
```

Store those handles as fields. Cloning shares interaction state. The host holds
weak references to open handles and the owning entity; it does not retain content
entities. Application code owns those entities even while an overlay is closed.
One handle must be mounted at most once, and all controls/anchors must belong to
the same host. A host binds to its first window; using it in another window
panics. Passing dialog state to a Popover, popover state to a Dialog, or an anchor
from another host also panics.

Mount `OverlayHost` at the window root. Its `body` contains normal application
content **and background key contexts/action handlers**. Overlay controls are
siblings of that body, keeping background shortcuts out of their focus path.
Avoid wrapping the host in background action handlers. Application-global action
handlers still need their own availability checks.

A render method can compose these components:

```rust
# use rake::gpui::{self, prelude::*};
# use rake::overlay::{Dialog, OverlayAnchor, OverlayHost, OverlayHostState, AnchorState, OverlayState, Popover};
# fn render(overlays: OverlayHostState, anchor: AnchorState, actions: OverlayState, documents: OverlayState) -> impl IntoElement {
let open_actions = actions.clone();
let body = rake::view! {
    <section class="p-4">
        <OverlayAnchor state={anchor.clone()}>
            <button aria-expanded={actions.is_open()}
                onclick={move |_, window, cx| { open_actions.open(window, cx); }}>
                "More actions"
            </button>
        </OverlayAnchor>
    </section>
}.into_any_element();
let open_documents = documents.clone();

rake::view! {
    <OverlayHost state={overlays} body={body}>
        <Popover state={actions} anchor={anchor}>
            <button onclick={move |_, window, cx| { open_documents.open(window, cx); }}>
                "Browse documents"
            </button>
        </Popover>
        <Dialog state={documents} title={"Documents".into()} footer={None}>
            <p>"Mount a retained entity here with a view tag."</p>
        </Dialog>
    </OverlayHost>
}
# }
```

`open(window, cx)` remembers the opener and returns false if already open or the
owner has been released. Initial focus enters the first enabled tab stop after
mount; an empty overlay focuses its container. `close(window, cx)` closes that
layer and everything opened above it. It restores a still-mounted opener when
focus belonged to the removed layers; otherwise it preserves the current focus.
If the opener disappeared, focus returns to a remaining overlay or the host's
mounted fallback.

Only the top layer handles Escape or outside pointer dismissal. A child can
consume Escape. Outside clicks dismiss without activating the covered content.
Popovers close when focus leaves; dialogs contain Tab/Shift+Tab and repair
programmatic focus escapes. A popover opened above a dialog inherits that modal
focus boundary. Disabled macro buttons are skipped. Sentinels also catch controls
that consume Tab and invoke GPUI's window-wide navigation themselves.

Anchors update during prepaint, including scroll and resize. Popovers are placed
below the visible trigger bounds and clamped to the window with a 16-point margin.
Scrolling an anchor fully out of view, removing the anchor, or removing an open
surface closes that layer and its descendants on the next frame. Keep the host
and anchor prepaint hooks outside explicitly cached subtrees unless you invalidate
those caches on every geometry change.

Dialogs use an accessible Dialog role, their visible title as their accessible
name, and modal metadata. The host's background accessibility group is hidden
while a dialog is open. Popovers are groups of controls: menu roles, arrow-key
selection, submenus and screen-reader announcement policies are not implemented.
Surfaces currently have fixed dark styling, widths of 320/520 points, viewport
limits and overflow scrolling; customizable skins and placement strategies can
follow actual usage.

## Tooltips and the example

Use a GPUI tooltip builder through `style`, returning a retained help entity:
`style={|s| s.tooltip(move |_, _| help.clone().into())}`. Keep essential information
in labels/descriptions as well; hover help is supplementary. No new tooltip
runtime is introduced.

Run `cargo run -p rake-view`. **More actions** opens a popover; **Browse 10,000
documents** opens a nested dialog with a virtual list. **Jump to 5,000** selects
and reveals that record. Click a row, then use Up/Down, Home or End. Closing and
reopening preserves selection and scroll because the Documents entity and its
scroll handle remain owned by the counter. Escape closes the dialog before the
popover and restores each opener. The actions trigger has retained GPUI hover help.

Frame tests exercise stack ordering, real keyboard/pointer dispatch, focus
restoration/containment, disabled controls, consumed events, layout/scroll anchor
updates, unmounting and bounded virtual rows. Their window/text/GPU backend is
GPUI's TestPlatform. Native and Web compilation are separate checks; these tests
do not establish browser or assistive-technology behavior.

The native smoke check verified keyboard opening of the nested dialog, stacked
Escape/restored opener activation, virtual-list selection and retained scroll
across reopening, and suppression of the background increment shortcut. Browser input
and screen-reader interaction have not been verified.
