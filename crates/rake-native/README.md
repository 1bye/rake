# rake-native

Optional native components in GPUI layouts. The first adapter is macOS AppKit.
Enable `rake`'s `native-components` feature and import `rake::native::macos`, or
depend on `rake-native` directly with the matching pinned GPUI-CE version.
The feature is off by default. Other targets compile the crate without Apple
dependencies and expose no macOS API; callers select their own portable controls
with `#[cfg(target_os = "macos")]`.

```toml
[dependencies]
rake = { path = "../rake/crates/rake", features = ["native-components"] }
```

Create controls once in a GPUI entity, then insert their `.element()` on each
render. AppKit supplies intrinsic sizing, drawing, input and accessibility.
Use GPUI's `Styled` methods to set layout dimensions, visibility and local opacity.

```rust
// Inside your entity's constructor:
let save = Button::new("Save", ButtonStyle::LiquidGlass)?;
let entity = cx.weak_entity();
save.on_click(cx, move |cx| {
    entity.update(cx, |this, cx| {
        this.save();
        cx.notify();
    }).ok();
});

// Inside Render::render:
div().child(self.save.element().w(px(120.)).h(px(32.)))
```

`Button::symbol` accepts an SF Symbol name and an accessible description.
`SegmentedControl` supplies native single selection, validates indices and
schedules `on_change` callbacks on GPUI's foreground executor. `as_appkit()`
allows additional native customization; leave attachment, target/action and
segment count to the adapter. Capture weak entities in callbacks so an entity
does not retain itself through its own control. Replacing a handler affects
future actions; actions already queued retain their original handler.

`NativeView::from_appkit` accepts an owned `objc2::rc::Retained<NSView>` for
custom native components. Matching bindings are re-exported as `macos::objc2`,
`macos::appkit` and `macos::foundation`. All view creation and use is on the
AppKit main thread. Clones share one view, which must occupy only one layout slot.
Removing an element or closing its window detaches its native host.

The bridge exposes the live GPUI content and native hosts as sibling accessibility
groups in the window. This keeps native controls discoverable despite GPUI's
AccessKit adapter replacing the content view's accessibility children. The adapter
replaces `accessibilityChildren` and `accessibilityHitTest:` at runtime on the
window's stable class and any overriding current KVO subclass. These replacements
last for the life of the process. Windows with no native hosts fall through to
the original implementation; hit testing preserves Base's GPUI canvas forwarder.
When the final host detaches, the adapter restores the content view's accessibility
state and resumes forwarding window queries to the original implementation.

## Composition boundaries

- Native views sit above GPUI's rendered surface. GPUI overlays, draw order,
  transforms, rounded masks and ancestor opacity groups cannot cover or modify
  them. Use separate, non-overlapping slots. Rectangular content-mask clipping
  and the native element's own visibility/opacity are supported.
- AppKit controls use AppKit keyboard focus and accessibility. GPUI focus handles,
  key bindings, element inspection and `rake::view!` button events do not represent
  native control input. Native controls are inserted in their first mount order;
  GPUI reordering is not native keyboard traversal ordering.
- `ButtonStyle::LiquidGlass` embeds a borderless button in `NSGlassEffectView`
  on macOS 26+, with an AppKit toolbar-button fallback on older systems. This
  provides native glass for the native control, not a glass backdrop behind GPUI
  children. Arbitrary GPUI backdrops require a renderer integration such as
  Photon's cutout primitive; the current GPUI pin is unchanged.

Run the facade example on macOS:

```sh
cargo run -p rake --example native --features native-components
```

Verification:

```sh
cargo test -p rake-native --doc
cargo clippy -p rake-native --all-targets -- -D warnings
cargo check -p rake --features native-components --example native
cargo check -p rake --features native-components --lib --target wasm32-unknown-unknown
```

The explicit native suite opens an actual GPUI window, exercises native actions,
clipping, removal, remounting and window teardown. It requires macOS with a
logged-in graphical session and a working GPU:

```sh
cargo test -p rake-native --features native-tests --test appkit
```
