# GPUI Base and optional Kit

Rake pins `gpui-pre`/`gpui-pre-platform` 0.3.11 and Base/Kit 0.9.0, using Rust
1.98.0. `experimental-view` enables Base-backed Rake controls. `kit` additionally
enables Kit and exposes its matching facade as `rake::kit`. Default Rake builds
compile neither Base nor Kit; Base-only views do not compile Kit.

```toml
rake = { path = "../rake/crates/rake", features = ["kit"] }
```

Use `rake::kit::component::button::Button`, `rake::kit::component::table`, and
`rake::kit::component::WindowExt` directly. The [routed example](../examples/kit/README.md)
uses Kit's button, dialog and table inside `rake::view!` and retained Rake tabs.
Run `cargo run -p rake-kit-example --features kit,devtools` on macOS.

## Window and focus ownership

Rake's application builder initializes Kit once, installs its default icon
assets, and creates the Base Root used by Kit's presentation plugin. Initial
and additional routed windows use this same root. There is one Base focus-trap
manager. Kit owns window-wide dialogs, sheets and notifications; no Kit root is
nested inside Rake's root.

Existing `rake::overlay` surfaces remain local, controlled overlays. While a Kit
global dialog or sheet is active they suspend rendering, missing-anchor audits
and focus guards, retaining their state. Kit can then take focus and return it
to the local opener when closed. This is covered by a real GPUI input/focus
regression test. Local Rake surfaces resume when Kit's modal is no longer active.
Programmatic local opens/closes also preserve Kit's focus. Closing a suspended
surface defers opener restoration until Kit closes; a removed opener uses the
mounted host fallback. Suspended popovers release Base's deferred popup context
so inputs inside the Kit dialog can use their own context menus.

For custom GPUI window construction, call `rake::init_ui` before creating the
Base Root and provide `rake::kit::assets::Assets` to GPUI’s application; the Rake `App` builder handles this itself.
If the host already initialized Kit, Rake adopts that initialization and preserves
its theme, registry and bindings.

## One semantic color write path

Declare `rake::theme!(rake::theme::Semantic)` for flat semantic color classes.
Existing arbitrary app theme structs remain supported independently.

```rust
let mut palette = rake::theme::Semantic::dark();
palette.primary = rake::gpui::rgb(0x3c8060).into();
rake::theme::install(cx, palette);
```

With Kit off, installation updates Rake's ambient copy and Base's colors. With
Kit on, it switches Kit's mode first, then applies the palette through
`Kit Theme::update`. Kit updates Base. A Base-global observer refreshes Rake's
copy from Kit's `semantic_tokens()` when Kit itself changes its theme. Equal
values skip replacement, preventing notification loops. Tests exercise actual
GPUI global effects for both paths, including a direct Kit mode change.

Base `surface`/`surface_foreground` map to Kit `popover`/`popover_foreground`;
Base `destructive`/`destructive_foreground` map to Kit `danger`/`danger_foreground`.
The bridge also writes `selection`, omitted by Kit 0.9's `apply_semantic_tokens`.
Semantic covers Base's 18 color roles and appearance. Kit's other component
colors follow its mode/theme; typography, spacing and elevation are separate
scales. When Kit is enabled, direct writes to Base are not a supported theme
write path because Kit rebuilds that global.

## Inspection boundaries

Wrap Kit builders with `.inspect("kit.button")` after styling. This records
geometry, construction sites and authored wrapper styles. Use explicit
`.inspect_handler("click")` declarations and pass a real Kit focus handle to
both the control's `.track_focus` and the inspection boundary's `.focus_handle`
when agents need focus navigation.

GPUI does not enumerate arbitrary component descendants. Rake cannot see or
edit Kit's private children, internal resolved styles, implicit focus handles,
or unregistered control values. These boundaries do not provide `view!` parity.
Rake-authored nodes inside Kit dialog content retain normal inspection and style
editing. With `kit,devtools`, a Root presentation decorator captures the full
window through the end of deferred painting, so global dialog boundaries appear
alongside routed page boundaries. A regression checks deferred paint, editability
and stable identity on rerender.

## Targets and pin checks

Core Rake and Base views build and run on stable single-threaded Web. Kit 0.9.0
is explicitly native-only in Rake: its published platform/Web dependencies enable
multithreading even when Kit defaults are disabled. An isolated upstream check
fails in `wasm_thread` with stable-channel `E0554`. Cargo features are additive;
a downstream `default-features = false` cannot turn that dependency feature off.
Enabling Rake's `kit` on Wasm produces a clear compile error. No nightly, fork or
vendored implementation is used. The browser-compatible example is `rake-view`.

macOS was run. Windows and Linux remain unverified.
`python3 scripts/check-gpui-pins.py` checks workspace and resolved ecosystem pins,
upstream engine requirements, default optionality and the stable Web graph. CI
also tests Kit's theme observer, modal coexistence and example compilation/lints.
See [the migration verification](gpui-pre-migration.md) for commands, browser
coverage and clean Wasm measurements.
