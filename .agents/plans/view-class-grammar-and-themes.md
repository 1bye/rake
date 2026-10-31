# Plan: Tailwind-style class grammar and themes for `rake::view!`

Handoff plan. The goal is to make `class="..."` expressive enough that most
`style={|s| ...}` closures disappear, with everything resolved at compile time
and good error messages.

Evidence for the goal: `examples/inspector/src/view.rs` has 84 style closures.
They contain 83 theme-color reads (`rgb(colors.x)`), 85 `px(..)` values,
15 `.hover(..)` calls and 92 `if` expressions.

Work the phases in order, one reviewable change per phase. Phase 1 is a
behavior-preserving refactor that the later phases build on.

## Settled design decisions (don't relitigate)

- **Classes stay string literals:** `class="..."`, plus the existing
  `class:<utility>={bool}`. An unquoted `class=[..]` token form is **out of
  scope**.
- **Themes are ambient and managed by Rake.** No per-component or
  per-invocation theme argument, and no reliance on a `cx` variable in scope.
  Components just write `class="bg-surface"`.
- **The theme type is the app's own struct**, declared once per crate. Rake
  provides no built-in theme vocabulary.
- **Theme values are fixed when the element is built**, not at layout. GPUI
  stores hover/focus/active styles in `pub(crate)` fields that can be set only
  once, while building: see the `debug_assert!(... "hover style already set")`
  in `gpui/src/elements/div.rs` around line 833. A later layout pass can't
  change them.
- **No `dark:` variant.** Dark mode means installing a different theme value.
- **Out of scope:** subtree theme overrides, a standard theme set shared across
  component libraries, a formatter, and reading theme config from files.

## Before you start

- Read `AGENTS.md`, `.agents/workflow/rust/AGENTS.md`, `architecture.md`,
  `testing.md` and `rustdoc.md`.
- Run `git status`. The owner may have uncommitted edits in `examples/inspector/**`
  and `docs/rake-inspector.md`. Never stage, revert or reformat them. Don't run
  plain `cargo fmt`; use `cargo fmt --all --check`, or `rustfmt --edition 2024
  <file>` on the files you changed.
- Read the current implementation:
  - `crates/rake-macros/src/view.rs`: parser, `apply_attributes`
  - `view/classes.rs`: `FIXED`, `SCALES`, `PREFIXES`, `utility_methods`,
    `class_methods`
  - `view/paragraph.rs`: `inline_classes`
  - `view/diagnostics.rs`: error recovery and probing
  - `view/tests.rs`: snapshots, and the `documented_utilities_match_the_checked_table`
    test, which reads the utility list between `<!-- view-utilities:start -->`
    and `<!-- view-utilities:end -->` in `docs/experimental-view.md`
- GPUI is `gpui_ce` at rev `99109e3`, source at
  `~/.cargo/git/checkouts/gpui-ce-*/99109e3/crates/`. Check every method you
  emit. Verified so far:
  - `rems`, `px`, `relative` (`gpui/src/geometry.rs`)
  - `rgb`, `rgba` (`color.rs`)
  - `opacity`, `line_height`, `font_family`, `text_size`, `text_color`
    (`styled.rs`)
  - `hover`, `active`, `focus`, `focus_visible`, `in_focus`, `group`,
    `group_hover`, `group_active` (`elements/div.rs`)
  - `impl Styled for StyleRefinement` (`style.rs:326`)
  - `App::set_global`, `global`, `observe_global`, `refresh_windows`
    (`app.rs`)

  Still to verify: `bg`'s signature, border-width and radius methods that take
  a length, inset methods (`top`, `left`, ...), and whether `FluentBuilder`
  (`when`) is implemented for `StyleRefinement`.

### Verification (every phase; run in bash, since the owner's shell is zsh)

```sh
cargo test -p rake-macros --locked
cargo test -p rake --features experimental-view --test view --test view_components --test view_paragraphs --test view_ui --locked
cargo test -p rake --features view-test-support --test view_state --locked
cargo test -p rake --features experimental-view --doc --locked
cargo check -p rake-view -p renamed-facade --all-targets --locked
cargo check -p rake-view -p renamed-facade --lib --target wasm32-unknown-unknown --locked
cargo clippy -p rake-macros -p rake --features experimental-view --all-targets --locked -- -D warnings
cargo fmt --all --check
```

**Rules for every phase:**
- Existing expansion snapshots and `.stderr` files stay byte-identical, unless
  the phase says otherwise. When a phase does allow a change, review each diff
  and explain it in your report.
- Update the `view!` rustdoc in `crates/rake-macros/src/lib.rs` and
  `docs/experimental-view.md`, including the utility list between the markers,
  in the same phase as the behavior change.

---

## Phase 1: Style plan IR (no behavior change)

**Goal:** one internal representation for everything a class can contribute,
so that variants, conditionals and themes can merge into **a single GPUI call
per variant**.

1. **Add `view/style_plan.rs`:**
   - `StylePlan { base: Vec<Segment>, variants: BTreeMap<Variant, Vec<Segment>> }`
   - `Segment::Always(TokenStream)` or
     `Segment::When { condition: Ident, then: TokenStream, otherwise: TokenStream }`
   - `Variant` is an enum. For now it only needs to exist; Phase 3 fills it.
2. **Route everything through the plan.** `class="..."` and
   `class:<utility>={cond}` both build `StylePlan` segments instead of emitting
   tokens directly.
3. **Conditions are bound once, in written order.**
   - Each `class:` condition becomes `let __rake_class_cond_N: bool = cond;`
     (a `mixed_site` ident) in the element's existing `setup` block.
   - Segments refer to that local.
   - This keeps the guide's rule that attribute expressions run once, in
     written order.
4. **Emission:**
   - Base segments emit at the position of their attribute, preserving today's
     "classes and style closures apply in attribute order".
   - `Always` emits `.method()`; `When` emits `.when(local, |s| s.method())`.
5. **Snapshots:**
   - The only expected snapshot diff is `conditional_classes.txt`, from the
     condition moving into a local.
   - If you can make the emission byte-identical instead, do. Otherwise review
     and explain the diff.
   - All other snapshots stay identical.

**Done when:** classes go through `StylePlan`, and the snapshot rules above
hold.

---

## Phase 2: Arbitrary values, open scale, negatives, fractions

Extend `utility_methods` (or its Phase 1 replacement) in `view/classes.rs`.

| Syntax | Expansion | Notes |
| --- | --- | --- |
| `<prefix>-N` with any N in steps of 0.5 (`p-13`, `gap-1.5`) | `.p(rems(N / 4))` | Only for values **not** in `SCALES`; scale values keep using the existing methods so snapshots stay identical |
| `<prefix>-[<n>px\|rem\|%]` | `.w(px(n))`, `.w(rems(n))`, `.w(relative(n / 100))` | The unit is required; `[320]` errors with "add px, rem or %" |
| `w-1/2`, `h-2/3`, `basis-1/3` (if a basis method exists) | `relative(a / b)` | Denominators 2, 3, 4, 5, 6 and 12; reject `/0` |
| `-mt-1`, `-mx-[4px]`, `-top-2` | negative length | Allowed **only** for margins and inset; reject negative padding, gap and size |
| `top-`, `right-`, `bottom-`, `left-`, `inset-` (scale or `[..]`) | the GPUI inset methods | Verify the method names |
| `text-[11px]`, `text-[0.75rem]` | `.text_size(..)` | A length inside `text-[..]` means size |
| `text-[#rrggbb]`, `bg-[#..]`, `border-[#..]` | `.text_color(rgb(0x..))` etc. | Also accept `#rgb` and `#rrggbbaa` (via `rgba`); validate the hex at compile time |
| `leading-[20px]`, `leading-none` | `.line_height(..)` | `leading-none` is `relative(1.)` |
| `opacity-0` … `opacity-100` (steps of 5), `opacity-[0.35]` | `.opacity(f32)` | Range 0..=1 |
| `font-[JetBrains_Mono]` | `.font_family("JetBrains Mono")` | Underscores become spaces, as in Tailwind |
| `rounded-[4px]`, `border-[2px]`, `border-t-[2px]` | radius and border-width methods taking a length | Verify the names |
| `min-w-[..]`, `max-w-[..]`, `min-h-[..]`, `max-h-[..]` | the matching methods | |

**Parsing:**
- Keep the parser small and table-driven: classify the prefix, then parse the
  value.
- Put value parsing in `view/classes/value.rs`, or a module in that spirit,
  with pure unit tests covering valid values, edge cases and every rejection.
- Emitted float literals must be suffixed (`320.0_f32`) so types are
  unambiguous.

**Inline paragraph classes** (`inline_classes` in `paragraph.rs`):
- Accept only text-run concepts: `text-[#..]`, `font-[..]`.
- Reject sizes, with the existing "belongs on p" message.

**Tests:**
- Extend the compile test in `crates/rake/tests/view.rs` that applies every
  utility to a real `div`, so each new form compiles against GPUI.
- Add one runtime test asserting the resulting `StyleRefinement` values, for
  example that `w-[320px]` gives width `px(320.)` and `w-1/2` gives
  `relative(0.5)`. Follow the existing style-assertion tests in `view.rs`.
- Add UI fixtures for: missing unit, bad hex, negative padding, `/0`, and
  out-of-range opacity.

---

## Phase 3: Variants

**Grammar:** `<variant>:<utility>`. Variants:

| Variant | GPUI call |
| --- | --- |
| `hover:` | `hover` |
| `active:` | `active` |
| `focus:` | `focus` |
| `focus-visible:` | `focus_visible` |
| `in-focus:` | `in_focus` |
| `group-hover/<name>:` | `group_hover(name, ..)` |
| `group-active/<name>:` | `group_active(name, ..)` |
| `disabled:` (buttons only) | base segments guarded by the button's existing disabled local |

`group/<name>` (no colon) is a plain utility: `.group("name")`.

1. **Merge per variant.**
   - Every `hover:` contribution on an element, from `class` and from
     `class:hover:bg-x={cond}`, goes into the plan's `hover` bucket.
   - Each bucket emits **exactly one** call after the element's authored
     attributes (alongside where focus configuration is applied today):
     `.hover(|s| { let s = s; <segments> s })`.
   - Inside it, `When` segments use `if local { s.x() } else { s }`, unless
     `FluentBuilder` works on `StyleRefinement`.
2. **Reject stacking** (`hover:active:bg-x`): GPUI refinements can't nest
   interaction states. Error: "variants can't be stacked; GPUI applies one
   interaction state per style block".
3. **Restrict what variants accept.** `StyleRefinement` implements `Styled`,
   but not every utility makes sense in a variant. Reject the ones that
   produce interactivity or identity (`group/..`, `cursor-*` if it fails to
   compile, and so on). Find the set by compiling every utility inside
   `.hover(..)` in a test; anything that doesn't compile is rejected with a
   clear message.
4. **Conflict with `style` closures.**
   - Scan the tokens of a `style={..}` value for a method call named `hover`,
     `active`, `focus`, `focus_visible`, `in_focus`, `group_hover` or
     `group_active`: a `.` followed by that ident and a parenthesized group.
   - If the element also has class variants of the same kind, report an error
     at the closure: "`hover:` classes and `.hover(..)` in style both set GPUI's
     single hover style; move one into the other".
   - Document that this is a token heuristic. GPUI's own debug assert remains
     the backstop.
5. **The button `disabled:` variant** emits
   `.when(__rake_view_disabled_N, |s| s.<utilities>)` with the existing local.
   On other tags it's an error.
6. **Tests:**
   - snapshots for merged variants
   - `view_state` frame tests showing a hovered element gets the hover color,
     and that two `hover:` classes on one element don't panic in debug
   - UI tests for stacking, the style-closure conflict, `disabled:` on a
     non-button, and `group-hover` without a name

---

## Phase 4: Literal-branch class expressions

**Accept** `class={EXPR}` where EXPR is, recursively:
- a string literal
- `if cond { EXPR } else { EXPR }`, including `else if` chains, with `else`
  optional (a missing `else` means no classes)
- `if let PAT = e { EXPR } else { EXPR }`
- `match e { PAT (if guard)? => EXPR, ... }`

**Behavior:**
- Each literal leaf is parsed with the full grammar.
- Variants inside branches merge into the variant buckets as conditional
  segments.
- Conditions and scrutinees are evaluated **once**, in written order:
  - Bind the `if` conditions to locals in `setup`.
  - For `match` and `if let`, evaluate the scrutinee once into a local and use
    `matches!(local, PAT if guard)` per arm. Arms are exclusive, so emit them
    as an `if`/`else if` chain over `matches!` checks.
  - **Patterns that bind variables are rejected**, since a binding can't be
    used inside a literal class string. The error suggests `_` or a plain
    comparison.

**Errors:**
- Anything else (a non-literal leaf such as a variable, `format!`, or a method
  call) keeps the current error, extended with: "class expressions may only
  choose between string literals".

**Tests:** snapshots, a runtime test that the right branch's styles apply, and
UI fixtures for a non-literal leaf and a binding pattern.

---

## Phase 5: Error quality

1. **"Did you mean" suggestions.**
   - Add a small edit-distance function (no new dependency) over all known
     utility names, prefixes and variants.
   - Suggest the closest match within distance 2:
     "unsupported utility `justify-betwen`; did you mean `justify-between`?"
2. **Name the position.** A string literal is one span on stable Rust, so
   errors can't underline a single class. Messages must quote the class and
   give its position: "3rd class `bg-red-50`".
3. **Messages per category**, for example:
   - "spacing values are whole or half steps (`p-4`, `p-1.5`) or `[value]`"
   - "colors come from your theme (`bg-surface`) or `[#rrggbb]`"
   - "unknown variant `hovr:`; did you mean `hover:`?"
4. **Report every bad class** in a string, combined, which `class_methods`
   already does. Keep it that way for variants and branches too.
5. **Tests:** update the existing stderr fixtures **only** where the message
   text intentionally improved, and review each change. Add fixtures for a
   typo suggestion, an unknown variant and a bad value per category.

---

## Phase 6: Theme runtime (in the `rake` crate)

**Add the `rake::theme` module** in `crates/rake/src/theme.rs`, behind
`experimental-view`, exported from `lib.rs`:

```rust
pub fn install<T: 'static>(cx: &mut gpui::App, theme: T);
pub fn current<T: 'static>() -> std::rc::Rc<T>;  // panics with guidance if absent
pub fn try_current<T: 'static>() -> Option<std::rc::Rc<T>>;
```

**Storage:**
- A `thread_local!` `RefCell<HashMap<TypeId, Rc<dyn Any>>>`. Several theme
  types can coexist; for example, a component crate with its own theme type
  next to the app's.
- `install` also:
  - sets a GPUI global `ThemeGlobal<T>(Rc<T>)`, so apps can
    `cx.observe_global::<ThemeGlobal<T>>()`
  - calls `cx.refresh_windows()`
- Document that `install` is the only supported way to change a theme. Setting
  the GPUI global directly would leave the macro's copy stale.

**Guidance on panic:**
- The message for a missing theme names the type and says to call
  `rake::theme::install(cx, ...)` during app setup.
- Rationale: GPUI's UI runs on one thread, wasm included, so a thread-local
  copy is safe. Document this assumption.

**Text values:** add `rake::theme::TextToken { size: Pixels, line_height:
Option<Pixels>, weight: Option<FontWeight> }`.

**Type-directed application traits** in `rake::__private`. They let one class
prefix accept several value types:
- `ThemeText`: implemented for any color, setting the text color, and for
  `TextToken`, setting size, line height and weight
- `ThemeBackground`: colors
- `ThemeBorder`: colors
- `ThemeFont`: `SharedString`, `&'static str`
- `ThemeRadius`: `Pixels`, `Rems`

Each is a trait with `fn apply<S: Styled>(self, s: S) -> S`. Use
`gpui::IntoColor`/`Hsla` bounds matching what `text_color` and `bg` accept;
check the exact traits in the GPUI source.

**Tests** (in `view_state` or a new `theme` test file): install, then read
back; replacing a theme is visible to `current`; two theme types coexist; the
missing-theme panic message; the global is observable.

---

## Phase 7: Theme in the macro

1. **Declare once per crate.** `rake::theme!(crate::ui::Colors);` is a
   `macro_rules!` in `rake` that expands to
   `#[doc(hidden)] pub(crate) type __RakeTheme = $path;` and must be used at
   the crate root. It must work through the renamed facade
   (`tests/renamed-facade`).
2. **`#[derive(rake::Theme)]`** in `rake-macros`. It does validation only:
   - Reject field names that collide with built-in utility words under any
     theme prefix. For example: `xs`, `sm`, `base`, `lg`, `xl`, `2xl`,
     `3xl`, `left`, `center`, `right`, `none`, `full`, `normal`, `medium`,
     `semibold`, `bold`, `t`, `b`, `l`, `r`, numbers, and anything else the
     Phase 2 and 3 tables reserve. Keep the reserved list next to the class
     tables, with a test that it stays in sync.
   - Require `Clone`.
   - Generate no runtime code beyond what's needed.
3. **Expansion:**
   - Any class whose value part isn't a known utility value, under the
     prefixes `bg-`, `text-`, `border-` (after side and width parsing),
     `font-` (after weight keywords) and `rounded-` (after the size keywords),
     is a theme name.
   - A hyphen means an underscore: `bg-surface-raised` reads `surface_raised`.
   - The first theme use in an invocation adds, once, at the top of the
     macro's output block:
     `let __rake_theme = #rake::theme::current::<crate::__RakeTheme>();`
     (a `mixed_site` ident).
   - Each use emits
     `.map(|s| #rake::__private::ThemeText::apply(__rake_theme.<field>.clone(), s))`,
     using the matching trait. Variants reuse the same binding inside their
     closures.
   - **Spans:** give the field ident the class string's span. Then a typo
     produces rustc's own "no field `bra` on type `Colors`" error, which lists
     the available fields, at the class string.
   - Nested `view!` invocations inside expressions each bind their own theme
     handle. That's acceptable; it's an `Rc` clone.
4. **Inline paragraph runs:**
   - Accept `text-<theme>` for color only, through a run-style trait.
   - Reject `TextToken` there with the "size belongs on p" message. This works
     through the trait: implement the run trait only for colors, and document
     the resulting compiler error.
5. **Errors:**
   - A missing `rake::theme!` declaration shows up as "cannot find type
     `__RakeTheme` in the crate root" at the class. Add a trybuild fixture for
     it, and a doc note quoting that error with its fix.
   - Also add fixtures for an unknown field and a wrong field type (for
     example `font-` on an `Hsla` field).
6. **Tests:**
   - snapshots with theme classes
   - a frame test: install a theme, render `bg-surface hover:bg-hover`, assert
     the colors, install a second theme, and assert the re-render picks it up
   - a component test: a `RenderOnce` component deep in the tree uses theme
     classes with nothing passed to it
   - `examples/view` uses a small theme, demonstrating `install` and a
     light/dark switch with an existing typed action

---

## Phase 8: `#[rake::style]` presets

**Attribute macro:** `#[rake::style("h-[24px] px-2 rounded-sm hover:bg-hover")] pub fn control();`

- Expands to `pub fn control<S: Styled + InteractiveElement>(s: S) -> S { ... }`
  using the same class pipeline, including variants, theme and errors.
- Use only the bound `S: Styled` when the preset has no variants.
- Usage: `style={control}`, or composed as
  `style={|s| control(s).w(px(10.))}`.

**Rules:**
- Presets can't contain conditional (`class:x={..}`) or branch forms. There's
  no element context for them.
- Document that a preset containing `hover:` conflicts with `hover:` classes on
  the same element, the same as style closures do.
- Phase 3's conflict scan can't see inside the preset, so document GPUI's
  debug assert as the backstop.

**Tests:** a compile test, a frame test for preset hover with a theme color,
and UI fixtures for a bad class inside a preset and a conditional form inside
a preset.

---

## Phase 9: Docs, examples and migration

1. **Docs.** Rewrite the "Attributes, styling and events" utility section of
   `docs/experimental-view.md` as a grammar reference covering: prefixes and
   values, arbitrary values, variants, branches, theme, presets, and the error
   examples. Add a "Themes" section covering declaration, derive, `install`,
   switching, the threading assumption, and the missing-declaration error.
   Update the rustdoc.
2. **`examples/view`:** migrate it to theme classes. It must still build for
   native and wasm.
3. **Inspector migration is gated.**
   - Run `git status -- examples/inspector docs/rake-inspector.md`.
   - If anything there is modified or untracked, **don't touch the
     inspector**. Report that the migration is blocked on the owner
     committing their work.
   - If it's clean: replace `style` closures with classes where the result is
     equivalent; use `rake::theme!` plus `derive(Theme)` for its color struct;
     and keep closures for anything the grammar can't express (computed
     widths, for example). Report the before/after closure count.

---

## Reporting (per phase)

Include:
- files changed
- snapshot and stderr diffs, with a justification for each
- new fixtures
- the verification results
- GPUI methods you verified, or replaced because the planned name didn't
  exist
- anything left unverified, such as native or browser interaction (call this
  out explicitly)

Commit only if asked, one commit per phase, in the repo's style. For example:
- `refactor(view): route classes through a style plan`
- `feat(view): add arbitrary class values`
- `feat(view): add interaction variants`
- `feat(view): add literal class branches`
- `feat(view): suggest corrections for class errors`
- `feat(theme): add ambient theme runtime`
- `feat(view): resolve theme classes`
- `feat(view): add style presets`
- `docs(view): document the class grammar and themes`

End each commit message with
`Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Don't push.
