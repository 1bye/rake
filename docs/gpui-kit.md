# GPUI Base and optional Kit

Rake pins `gpui-pre`/`gpui-pre-platform` 0.3.11 and Base/Kit 0.9.0, using Rust
1.98.0. The default `gpui` feature includes Base-backed Rake controls. `kit` additionally
enables Kit and exposes its matching facade as `rake::kit`. Default Rake builds
compile Base but exclude Kit; headless `--no-default-features` builds exclude both.

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

Existing `rake::overlay` surfaces remain local, controlled overlays. While a Kit global dialog or sheet is active, they suspend rendering, missing-anchor audits and focus guards, and retain their state. Kit can then take focus and return it to the local opener when closed.

- Local Rake surfaces resume when Kit's modal is no longer active.
- Programmatic local opens and closes preserve Kit's focus.
- Closing a suspended surface defers opener restoration until Kit closes. A removed opener uses the mounted host fallback.
- Suspended popovers release Base's deferred popup context, so inputs inside the Kit dialog can use their own context menus.

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

- **Kit off:** installation updates Rake's ambient copy and Base's colors.
- **Kit on:** it switches Kit's mode first, then applies the palette through Kit's `Theme::update`, and Kit updates Base.
- **Kit changes its own theme:** a Base-global observer refreshes Rake's copy from Kit's `semantic_tokens()`.

Equal values skip replacement, which prevents notification loops.

Name mapping:

| Base | Kit |
| --- | --- |
| `surface`, `surface_foreground` | `popover`, `popover_foreground` |
| `destructive`, `destructive_foreground` | `danger`, `danger_foreground` |

The bridge also writes `selection`, which Kit 0.9's `apply_semantic_tokens` omits. `Semantic` covers Base's 18 color roles and appearance. Kit's other component colors follow its mode and theme, and typography, spacing and elevation are separate scales.

With Kit enabled, don't write directly to Base, because Kit rebuilds that global.

## Inspection boundaries

Wrap Kit builders with `.inspect("kit.button")` after styling. This records
geometry, construction sites and authored wrapper styles. Use explicit
`.inspect_handler("click")` declarations and pass a real Kit focus handle to
both the control's `.track_focus` and the inspection boundary's `.focus_handle`
when agents need focus navigation.

GPUI doesn't enumerate arbitrary component descendants, so Rake can't see or edit Kit's private children, internal resolved styles, implicit focus handles or unregistered control values. These boundaries don't match `view!`.

Rake-authored nodes inside Kit dialog content keep normal inspection and style editing. With `kit,devtools`, a Root presentation decorator captures the full window through the end of deferred painting, so global dialog boundaries appear alongside routed page boundaries.

## Targets and pin checks

Core Rake and Base views build and run on stable, single-threaded Web. **Kit 0.9.0 is native-only in Rake**, because its web dependencies enable multithreading even with Kit's defaults disabled, and that needs nightly (`wasm_thread` fails with `E0554` on stable). Cargo features are additive, so a downstream `default-features = false` can't turn it off.

Enabling Rake's `kit` on Wasm gives a clear compile error. The browser-compatible example is `rake-view`.

macOS was run. Windows and Linux remain unverified.
`python3 scripts/check-gpui-pins.py` checks workspace and resolved ecosystem pins,
upstream engine requirements, default optionality and the stable Web graph. CI
also tests Kit's theme observer, modal coexistence and example compilation/lints.
