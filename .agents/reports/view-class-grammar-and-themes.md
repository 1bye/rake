# View class grammar and themes: implementation report

Implemented all nine planned phases, one commit per phase. No push was performed. The owner's untracked `.agents/plans/` files were left untouched.

## Verification scope

Every phase ran the eight commands in the plan, plus the example's complete-text tests and the repository Dylint spacing check. Commands used locked dependencies; native and Wasm compile checks passed. Phase 7 Clippy found an unnecessary allocation; it was corrected and Clippy plus macro tests were rerun successfully. Phase 9 also ran inspector native/core tests, native/Wasm checks and Clippy for the migrated examples. The inspector loopback tests initially needed permission to bind local listeners outside the sandbox; their final rerun passed.

Frame tests use real GPUI construction, layout and pointer dispatch with GPUI TestPlatform window/text/GPU doubles. Theme tests observe inherited text colors and base background refinements across replacements. They do not establish rendered pixel colors, a live accessibility adapter, or native/browser interaction. No new native GUI or browser smoke session was performed in any phase. The existing complete-text tests compare distinct full strings; they do not check pixel output, ordering or duplicate occurrences.

## GPUI API findings

- Verified against pinned GPUI CE `99109e3`: lengths, insets, border/radius setters, text/opacity/font setters, interaction styles, globals/observation and window refresh.
- Basis uses `flex_basis`; fractions use `relative`. Border widths and radii take absolute px/rem lengths.
- `bg` takes `Into<Fill>`; text/border colors use Palette `IntoColor<Hsla>`. Theme helpers use those exact bounds. Separate inference markers keep color and TextToken implementations disjoint.
- `StyleRefinement` is Styled, but has no FluentBuilder. Conditional refinements use ordinary Rust if/else. Generated theme map operations become direct function application in refinement/run/preset contexts.
- Hover, named group-hover and focus/in-focus/focus-visible need InteractiveElement. Active, group-active and scrolling need StatefulInteractiveElement. Presets infer these bounds.
- Overflow scrolling and group declarations are rejected inside refinement variants. Cursor-pointer compiles there. SVG interaction styles require a surrounding interactive element.
- GPUI stores one named group per group-variant kind. Conflicting names are rejected. Literal style-call detection is heuristic; named presets remain opaque and GPUI assertions are the backstop.

## Phases

### 1. Style plan IR — `eedc44a`

Only `conditional_classes.txt` changes: condition and preceding builder bindings preserve attribute evaluation order. Other pre-existing snapshots and stderr files are unchanged.

Evaluation-order regression test; no new UI fixtures.

Verification: the complete phase matrix and spacing checks passed. See the shared scope above for interaction limits.

Changed files:

- `crates/rake-macros/src/lib.rs`
- `crates/rake-macros/src/view.rs`
- `crates/rake-macros/src/view/classes.rs`
- `crates/rake-macros/src/view/paragraph.rs`
- `crates/rake-macros/src/view/snapshots/conditional_classes.txt`
- `crates/rake-macros/src/view/style_plan.rs`
- `crates/rake/tests/view.rs`
- `docs/experimental-view.md`

### 2. Arbitrary values — `c0be4e4`

No pre-existing snapshot or stderr changes.

Missing-unit, bad-hex, negative-padding, zero-denominator and opacity-range UI fixtures; real GPUI style comparison and pure parser tests.

Verification: the complete phase matrix and spacing checks passed. See the shared scope above for interaction limits.

Changed files:

- `crates/rake-macros/src/lib.rs`
- `crates/rake-macros/src/view.rs`
- `crates/rake-macros/src/view/classes.rs`
- `crates/rake-macros/src/view/classes/value.rs`
- `crates/rake-macros/src/view/paragraph.rs`
- `crates/rake-macros/src/view/tests.rs`
- `crates/rake/tests/ui/view/arbitrary_bad_hex.rs`
- `crates/rake/tests/ui/view/arbitrary_bad_hex.stderr`
- `crates/rake/tests/ui/view/arbitrary_missing_unit.rs`
- `crates/rake/tests/ui/view/arbitrary_missing_unit.stderr`
- `crates/rake/tests/ui/view/fraction_zero.rs`
- `crates/rake/tests/ui/view/fraction_zero.stderr`
- `crates/rake/tests/ui/view/negative_padding.rs`
- `crates/rake/tests/ui/view/negative_padding.stderr`
- `crates/rake/tests/ui/view/opacity_range.rs`
- `crates/rake/tests/ui/view/opacity_range.stderr`
- `crates/rake/tests/view.rs`
- `crates/rake/tests/view_paragraphs.rs`
- `docs/experimental-view.md`

### 3. Interaction variants — `00df045`

Adds `variants.txt` and `disabled_classes.txt`; no pre-existing snapshot or stderr changes.

Seven variant UI cases; actual pointer-dispatch/frame checks and compile coverage for refinement-safe utilities.

Verification: the complete phase matrix and spacing checks passed. See the shared scope above for interaction limits.

Changed files:

- `crates/rake-macros/src/lib.rs`
- `crates/rake-macros/src/view.rs`
- `crates/rake-macros/src/view/classes.rs`
- `crates/rake-macros/src/view/snapshots/disabled_classes.txt`
- `crates/rake-macros/src/view/snapshots/variants.txt`
- `crates/rake-macros/src/view/style_plan.rs`
- `crates/rake-macros/src/view/tests.rs`
- `crates/rake/tests/ui/view/variant_conflict.rs`
- `crates/rake/tests/ui/view/variant_conflict.stderr`
- `crates/rake/tests/ui/view/variant_disabled.rs`
- `crates/rake/tests/ui/view/variant_disabled.stderr`
- `crates/rake/tests/ui/view/variant_group_conflict.rs`
- `crates/rake/tests/ui/view/variant_group_conflict.stderr`
- `crates/rake/tests/ui/view/variant_group_name.rs`
- `crates/rake/tests/ui/view/variant_group_name.stderr`
- `crates/rake/tests/ui/view/variant_group_style.rs`
- `crates/rake/tests/ui/view/variant_group_style.stderr`
- `crates/rake/tests/ui/view/variant_interactive.rs`
- `crates/rake/tests/ui/view/variant_interactive.stderr`
- `crates/rake/tests/ui/view/variant_stacking.rs`
- `crates/rake/tests/ui/view/variant_stacking.stderr`
- `crates/rake/tests/view.rs`
- `crates/rake/tests/view_state.rs`
- `crates/rake/tests/view_state/classes.rs`
- `docs/experimental-view.md`

### 4. Literal class branches — `8c47efa`

Adds `class_if.txt`, `class_if_let.txt`, `class_match.txt`. Existing `dynamic_class.stderr` only gains the planned literal-leaf explanation.

Binding-pattern, nonliteral-leaf and nonexhaustive-match UI fixtures; runtime single-evaluation, ordering and lazy-branch tests; hover rerender coverage.

Verification: the complete phase matrix and spacing checks passed. See the shared scope above for interaction limits.

Changed files:

- `crates/rake-macros/src/lib.rs`
- `crates/rake-macros/src/view.rs`
- `crates/rake-macros/src/view/classes.rs`
- `crates/rake-macros/src/view/classes/branch.rs`
- `crates/rake-macros/src/view/paragraph.rs`
- `crates/rake-macros/src/view/snapshots/class_if.txt`
- `crates/rake-macros/src/view/snapshots/class_if_let.txt`
- `crates/rake-macros/src/view/snapshots/class_match.txt`
- `crates/rake-macros/src/view/style_plan.rs`
- `crates/rake-macros/src/view/tests.rs`
- `crates/rake/tests/ui/view/class_binding_pattern.rs`
- `crates/rake/tests/ui/view/class_binding_pattern.stderr`
- `crates/rake/tests/ui/view/class_nonexhaustive.rs`
- `crates/rake/tests/ui/view/class_nonexhaustive.stderr`
- `crates/rake/tests/ui/view/class_nonliteral_leaf.rs`
- `crates/rake/tests/ui/view/class_nonliteral_leaf.stderr`
- `crates/rake/tests/ui/view/dynamic_class.stderr`
- `crates/rake/tests/view.rs`
- `crates/rake/tests/view_paragraphs.rs`
- `crates/rake/tests/view_state/classes.rs`
- `docs/experimental-view.md`

### 5. Class diagnostics — `8d93739`

No expansion snapshot changes. Existing class-error stderr messages gain ordinal/class context; unsupported-color text gains category guidance. Source spans are unchanged. Every changed stderr diff was reviewed.

`class_diagnostics` covers typo/variant suggestions, category errors, all branch leaves and inline errors together.

Verification: the complete phase matrix and spacing checks passed. See the shared scope above for interaction limits.

Changed files:

- `crates/rake-macros/src/lib.rs`
- `crates/rake-macros/src/view/classes.rs`
- `crates/rake-macros/src/view/classes/error.rs`
- `crates/rake-macros/src/view/paragraph.rs`
- `crates/rake-macros/src/view/style_plan.rs`
- `crates/rake/tests/ui/view/arbitrary_bad_hex.stderr`
- `crates/rake/tests/ui/view/arbitrary_missing_unit.stderr`
- `crates/rake/tests/ui/view/class_diagnostics.rs`
- `crates/rake/tests/ui/view/class_diagnostics.stderr`
- `crates/rake/tests/ui/view/fraction_zero.stderr`
- `crates/rake/tests/ui/view/handler_context.stderr`
- `crates/rake/tests/ui/view/multiple_errors.stderr`
- `crates/rake/tests/ui/view/negative_padding.stderr`
- `crates/rake/tests/ui/view/opacity_range.stderr`
- `crates/rake/tests/ui/view/unsupported_utility.stderr`
- `crates/rake/tests/ui/view/variant_disabled.stderr`
- `crates/rake/tests/ui/view/variant_group_conflict.stderr`
- `crates/rake/tests/ui/view/variant_group_name.stderr`
- `crates/rake/tests/ui/view/variant_group_style.stderr`
- `crates/rake/tests/ui/view/variant_interactive.stderr`
- `crates/rake/tests/ui/view/variant_stacking.stderr`
- `docs/experimental-view.md`

### 6. Theme runtime — `6cae2e4`

No snapshot or stderr changes. Cargo.lock only adds the already-resolved Palette package as a direct Rake dependency.

Install/read/replace/coexist/missing-theme/global-observer tests, no Clone requirement on stored values, and real Styled refinement application.

Verification: the complete phase matrix and spacing checks passed. See the shared scope above for interaction limits.

Changed files:

- `Cargo.lock`
- `crates/rake-macros/src/lib.rs`
- `crates/rake/Cargo.toml`
- `crates/rake/src/lib.rs`
- `crates/rake/src/theme.rs`
- `crates/rake/src/theme/apply.rs`
- `crates/rake/tests/view.rs`
- `crates/rake/tests/view_state.rs`
- `crates/rake/tests/view_state/theme.rs`
- `docs/experimental-view.md`

### 7. Theme classes — `bf3794d`

Adds `theme.txt`. The former unsupported `bg-red-500` fixture now uses `unknown-utility`: the original spelling can legitimately address a theme field. All other pre-existing snapshots/stderr stay unchanged. The example text fixture adds only `Switch theme`.

Six theme UI cases: missing declaration, unknown field, wrong type, reserved name, Clone, inline TextToken. Deep RenderOnce/theme replacement/hover frame coverage, every prefix compile coverage and renamed-facade native/Wasm checks.

Verification: the complete phase matrix and spacing checks passed. See the shared scope above for interaction limits.

Changed files:

- `crates/rake-macros/src/lib.rs`
- `crates/rake-macros/src/view.rs`
- `crates/rake-macros/src/view/classes.rs`
- `crates/rake-macros/src/view/paragraph.rs`
- `crates/rake-macros/src/view/snapshots/theme.txt`
- `crates/rake-macros/src/view/style_plan.rs`
- `crates/rake-macros/src/view/tests.rs`
- `crates/rake-macros/src/view/theme.rs`
- `crates/rake/src/lib.rs`
- `crates/rake/src/theme/apply.rs`
- `crates/rake/tests/ui/view/theme_clone.rs`
- `crates/rake/tests/ui/view/theme_clone.stderr`
- `crates/rake/tests/ui/view/theme_inline_size.rs`
- `crates/rake/tests/ui/view/theme_inline_size.stderr`
- `crates/rake/tests/ui/view/theme_missing.rs`
- `crates/rake/tests/ui/view/theme_missing.stderr`
- `crates/rake/tests/ui/view/theme_reserved.rs`
- `crates/rake/tests/ui/view/theme_reserved.stderr`
- `crates/rake/tests/ui/view/theme_unknown.rs`
- `crates/rake/tests/ui/view/theme_unknown.stderr`
- `crates/rake/tests/ui/view/theme_wrong_type.rs`
- `crates/rake/tests/ui/view/theme_wrong_type.stderr`
- `crates/rake/tests/ui/view/unsupported_utility.rs`
- `crates/rake/tests/ui/view/unsupported_utility.stderr`
- `crates/rake/tests/view.rs`
- `crates/rake/tests/view_state.rs`
- `crates/rake/tests/view_state/theme.rs`
- `docs/experimental-view.md`
- `examples/view/src/lib.rs`
- `examples/view/src/tests/text/common.txt`
- `tests/renamed-facade/src/lib.rs`

### 8. Style presets — `2353319`

No existing snapshot/stderr changes.

Three preset UI cases: bad class, conditional syntax, button-only disabled. Typed composition, generic Styled refinement, plain-Div focus, renamed facade and themed-hover frame tests.

Verification: the complete phase matrix and spacing checks passed. See the shared scope above for interaction limits.

Changed files:

- `crates/rake-macros/src/lib.rs`
- `crates/rake-macros/src/view.rs`
- `crates/rake-macros/src/view/preset.rs`
- `crates/rake-macros/src/view/style_plan.rs`
- `crates/rake/src/lib.rs`
- `crates/rake/tests/ui/view/preset_bad_class.rs`
- `crates/rake/tests/ui/view/preset_bad_class.stderr`
- `crates/rake/tests/ui/view/preset_conditional.rs`
- `crates/rake/tests/ui/view/preset_conditional.stderr`
- `crates/rake/tests/ui/view/preset_disabled.rs`
- `crates/rake/tests/ui/view/preset_disabled.stderr`
- `crates/rake/tests/view.rs`
- `crates/rake/tests/view_state/theme.rs`
- `docs/experimental-view.md`
- `tests/renamed-facade/src/lib.rs`

### 9. Docs and migration

No existing expansion snapshot, stderr or displayed-text fixture changes. Adds the `theme_reserved_scale` stderr fixture for a validation edge found during final review.

Adds a light-theme scene to the example full-text regression and installs the theme in the standalone tooltip test setup. Tightens Theme validation for open side-border widths (`t_13` collides with `border-t-13`), with a table-derived check and regression fixture.

Verification: the complete phase matrix and spacing checks passed. See the shared scope above for interaction limits.

Changed files:

- `crates/rake-macros/src/lib.rs`
- `crates/rake-macros/src/view/classes.rs`
- `crates/rake-macros/src/view/theme.rs`
- `crates/rake/src/lib.rs`
- `docs/experimental-view.md`
- `docs/rake-inspector.md`
- `examples/inspector/src/appearance.rs`
- `examples/inspector/src/input.rs`
- `examples/inspector/src/lib.rs`
- `examples/inspector/src/view.rs`
- `examples/view/src/components.rs`
- `examples/view/src/documents.rs`
- `examples/view/src/lib.rs`
- `examples/view/src/tests/mod.rs`
- `crates/rake/tests/ui/view/theme_reserved_scale.rs`
- `crates/rake/tests/ui/view/theme_reserved_scale.stderr`
- `.agents/reports/view-class-grammar-and-themes.md`

## Final migration notes

The inspector gate was checked clean before editing `examples/inspector/**` or `docs/rake-inspector.md`. Its palette fields changed from integer hex codes to GPUI Rgba values, preserving both original palettes. Icon painting and the explicit-palette input renderer use those typed values. Startup and the theme toggle install the ambient theme.

Inspector `view.rs` style closures: **84 → 10** (counting `style={|...| ...}`). Toolbar controls also use `style={control}`, the shared focus preset. Remaining closures cover computed pane/row sizing, captured swatches, tree semantics, an underline, resize cursor/focus, and the explicit-palette property helper. That helper keeps its construction-only identity test independent of ambient setup.

The main view example now uses theme classes throughout controls and text; only its component-children builder needs a style closure. Disabled opacity, selected document rows and badge appearance use class syntax. Its light/dark action and shortcut remain typed GPUI actions.

No dependency versions were upgraded. No new macro formatting, subtree themes, global theme vocabulary, CSS cascade, or dark variant was introduced.
