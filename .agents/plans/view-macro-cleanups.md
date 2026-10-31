# Plan: `rake::view!` follow-up cleanups

Follow-up to `.agents/plans/view-macro-improvements.md`, which was implemented as
commits `92a1060..ff31c46`. A review of those commits found no blocking issues,
only the cleanups below. They are small and independent, so one commit is fine;
two commits are also fine: macro code (tasks 1–5) and docs (tasks 6–7).

The plan aims for **no behavior change** beyond task 1, which only adds
diagnostics.

## Before you start

- Read `AGENTS.md` and `.agents/workflow/rust/AGENTS.md`. Follow their spacing
  and verification rules.
- Run `git status`. The working tree contains the owner's uncommitted edits in
  `docs/rake-inspector.md` and `examples/inspector/src/{appearance,input,tree,view}.rs`.
  None of these tasks touch those files. Do not stage, revert or reformat them.
- `.agents/plans/` is untracked; leave it untracked.
- Macro code lives in `crates/rake-macros/src/view.rs` and
  `crates/rake-macros/src/view/{diagnostics,paragraph,tag,events,classes,component}.rs`.
  Expansion snapshots live in `crates/rake-macros/src/view/snapshots/`, and
  compiler fixtures in `crates/rake/tests/ui/view/`.

### Verification (run after the tasks, and report the results)

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

Use bash, or check each exit code yourself; the owner's shell is zsh.

**Acceptance for tasks 2–5:**
- Every expansion snapshot is byte-identical.
- Every `.stderr` fixture is unchanged.
- Do not regenerate snapshots or stderr files to make tests pass. A diff means
  the refactor changed behavior.

---

## Task 1: Keep collected diagnostics when final emission fails

**Where:** `View::expand_resilient` in `view/diagnostics.rs` (around line 24).

**Problem:**
- `self.0.recover(...)` collects every local error into `diagnostics`.
- Then `let root = self.emit(rake)?;` returns early if `emit` fails, throwing
  the collected errors away.
- Today every error is caught during recovery, so this is latent. If a future
  check exists only in `emit`, the user would see one error instead of all of
  them.

**Change:**
- When `emit` fails, combine its error into `diagnostics`, using
  `Diagnostics::push`.
- Return `Ok(diagnostics.to_compile_error())` with no fallback block, since
  there is no valid builder to keep.
- Apply the same combining in the `#[cfg(test)] expand` helper, so unit tests
  see every error.

**Test:** add a unit test in `view/tests.rs` that calls the helper directly.
Construct a `Diagnostics` holding one error, simulate an emission error, and
assert the combined `syn::Error` iterates over both. If that needs a small
helper such as `fn finish(diagnostics, result) -> syn::Result<TokenStream>`,
extract it and test that. Don't invent an artificial markup case.

---

## Task 2: Move validation tables onto `Tag`

**Problem:** the same facts are written out in several places.

| Fact | Locations |
| --- | --- |
| Inline tag names `span`/`strong`/`em`/`br` | `diagnostics.rs` (around line 81), `paragraph.rs` `validate_heading_children` (around line 65), `paragraph.rs` `inline_element` |
| Required attributes (`img`: `src`+`alt`; `svg`: `path`; `view`: `entity`) and their messages | `diagnostics.rs` (around line 120 and around line 212), `view.rs` `base_builder` (`Tag::Svg`, `Tag::Image`), `view.rs` `expand_entity` |
| Leaf tags that reject children (`img`, `svg`, `hr`, `view`) and their messages | `diagnostics.rs` (around line 138), `view.rs` `base_builder`, `expand_entity` |
| The heading and paragraph rejection messages | `diagnostics.rs` and `paragraph.rs` |

**Change**, in `view/tag.rs`:
- `pub fn is_inline_name(path: &syn::Path) -> bool`: one definition of the
  inline tag set (`span`, `strong`, `em`, `br`). Keep in mind that standalone
  `span` resolves to `Tag::Container`, so the check must stay name-based.
- `pub fn required_attributes(self) -> &'static [(&'static str, &'static str)]`,
  pairing each attribute name with its error message. Messages must stay
  exactly as they are, for example "img requires src", "img requires alt text",
  "svg requires path" and "view requires entity={&stored_entity}".
- `pub fn leaf_message(self) -> Option<&'static str>`: the "cannot have
  children" message for leaf tags.
- `pub(super) const HEADING_CONTENT: &str` and `PARAGRAPH_CONTENT: &str` for
  the two rejection messages, or a `fn inline_rejection(heading: bool) -> &'static str`.

Replace every duplicate with these helpers. The order in which errors are
checked and emitted must not change. The stderr fixtures check this, including
`multiple_errors`, `image_alt`, `svg_path`, `svg_children`, `rule_children`,
`heading_block` and `paragraph_block`.

---

## Task 3: Remove redundant heading validation

**Where:** `validate_heading_children` in `view/paragraph.rs`, and its call at
the top of `expand_paragraph`.

**Why:**
- `Element::recover` already runs over headings with `inline = Some(true)`. It
  rejects non-inline children and replaces them with an empty `span` before
  `expand_paragraph` runs.
- Both the proc-macro path (`expand_resilient`) and the test helper (`expand`)
  call `recover` first.

**Change:**
- Before deleting, prove the function is redundant: temporarily make it
  `unreachable!()` on error and run the full verification suite. Revert that
  probe, then delete the function and its call.
- If any test reaches it, keep it, don't delete, and say why in your report.

---

## Task 4: Remove the dead disabled-gating branch

**Where:** `apply_attributes` in `view.rs`, near the end of the loop:

```rust
let method = if is_button && attribute.name.starts_with("on") {
    quote!(.when(!#disabled, |#mapped| #mapped #method))
} else {
    method
};
```

**Why:**
- Every `on:*` name and every legacy alias is handled earlier by the
  `events::is_event` branch, which `continue`s.
- Typed actions are handled by the `attribute.action` branch.
- Any other name starting with `on` ends in the `_ =>` arm, which returns an
  error. So this condition can never be true when reached.

**Change:** delete the conditional and extend `method` directly.

**Check:** the snapshots for `button`, `events` and `actions` are unchanged.

---

## Task 5: Resolve semantic roles into the tag

**Where:** `Tag::semantic_role(self, name: &str)` in `tag.rs`, which ignores
`self`, and its use in `base_builder` (`view.rs`, `Tag::Container` arm).

**Change:**
- Make it `Container(Option<&'static str>)`: the AccessKit role identifier,
  resolved in `Tag::resolve` for `nav`, `header`, `footer`, `main`, `aside` and
  `article`.
- `base_builder` reads the role from the variant.
- Update every `Tag::Container` match and comparison. The `== Tag::Container`
  checks in `diagnostics.rs` (`invalid_component` recovery) and `clear_as`
  become `matches!(.., Tag::Container(_))`, or recovery sets `Container(None)`.
- Delete `semantic_role`.

**Check:** the `semantic_elements` and `containers` snapshots are unchanged.

---

## Task 6: Move the process report out of user-facing docs

- `docs/view-macro-improvements-report.md` records how the previous agent
  worked; it isn't product documentation.
- Move it to `.agents/reports/view-macro-improvements.md` with `git mv`, and
  keep it tracked unless the owner says otherwise.
- Grep for references to its old path (`docs/`, `README.md`, `crates/`) and
  update them.

---

## Task 7: Correct the verification claims in the guide

**Where:** "Example and checks" in `docs/experimental-view.md` (around lines
666–675, starting "The updated example was smoke-tested on macOS…" and "…was
checked on macOS for…").

**Problem:**
- The paragraphs describe manual macOS smoke tests from iterations before the
  recent phases.
- The new features had no native or browser smoke test: canonical and
  right/middle/hover/scroll/drag/drop events, `tooltip`, `svg`, semantic
  containers, `hr`, heading composition, `match`, shorthand and defaults.

**Change:**
- Keep the historical facts, but date-scope them, for example "Before the
  canonical-event and element additions, the example was smoke-tested on
  macOS: …".
- Add one sentence saying the later additions are covered by compile, unit,
  UI and TestPlatform frame tests only, without native or browser
  interaction checks.
- Don't claim any new manual testing.

---

## Out of scope (don't do these)

- Changing the deprecation constant path shown in warnings
  (`rake::__private::view_legacy::ONCLICK`). It's cosmetic, and the note
  already says what to use.
- Migrating inspector `style` closures to the new utilities: those files have
  the owner's uncommitted edits.
- A `view!` formatter.

## Reporting

List:
- each task's files
- confirmation that snapshots and stderr are unchanged, or the exact intended
  diff for task 1
- task 3's redundancy proof
- the verification results, including anything you couldn't run

Commit only if the person running you asked for commits. Use the repo's style,
for example `refactor(view): consolidate tag validation tables` and
`docs(view): scope historical smoke-test claims`.
