# Plan: `rake::view!` review nits and commit

Follow-up to `.agents/plans/view-macro-cleanups.md`. Those cleanups are done but
**uncommitted** in the working tree, and a review approved them. This plan
fixes two readability nits found in that review, then commits the cleanup work.

There must be **no behavior change**: every expansion snapshot and UI `.stderr`
file stays byte-identical.

## Before you start

- Read `AGENTS.md` and `.agents/workflow/rust/AGENTS.md`. Follow their spacing
  rules.
- Run `git status`. You'll see several groups of changes:
  - The cleanup work, which this plan commits:
    - `crates/rake-macros/src/view.rs`
    - `crates/rake-macros/src/view/{diagnostics,paragraph,tag,tests}.rs`
    - `docs/experimental-view.md`
    - the staged rename `docs/view-macro-improvements-report.md` →
      `.agents/reports/view-macro-improvements.md`
  - **The owner's own uncommitted work — never stage, revert or reformat:**
    `docs/rake-inspector.md` and
    `examples/inspector/src/{appearance,input,tree,view}.rs`.
  - `.agents/plans/`, which is untracked: leave it untracked.
- Don't run `cargo fmt` without `--check`. It would rewrite the owner's
  inspector files. If formatting is needed, run
  `rustfmt --edition 2024 <file>` on the specific files you changed.

## Nit 1: Simplify the early return in `expand_resilient`

**Where:** `View::expand_resilient` in
`crates/rake-macros/src/view/diagnostics.rs` (around lines 14–22).

Currently:

```rust
let root = match self.emit(rake) {
    Ok(root) => root,
    Err(error) => {
        return Ok(diagnostics
            .finish(Err(error))
            .unwrap_or_else(|errors| errors.to_compile_error()));
    }
};
```

`finish(Err(..))` always returns `Err`, so the `unwrap_or_else` only ever
turns the combined error into tokens. Write that directly:

```rust
let root = match self.emit(rake) {
    Ok(root) => root,
    Err(error) => {
        diagnostics.push(error);

        return Ok(diagnostics
            .0
            .expect("emission error was just recorded")
            .to_compile_error());
    }
};
```

Alternatively, add a small `Diagnostics::into_compile_error(self) -> TokenStream`
next to `finish`/`push` in `crates/rake-macros/src/view.rs`, and use it both
here and in the existing `if let Some(errors) = diagnostics.0` branch below.
Choose whichever reads more simply; don't add both.

**Keep:**
- `Diagnostics::finish`: the `#[cfg(test)] expand` helper uses it.
- `final_emission_errors_keep_previously_collected_diagnostics` in
  `view/tests.rs`.

## Nit 2: Name `em` explicitly in `inline_element`

**Where:** `inline_element` in `crates/rake-macros/src/view/paragraph.rs`
(around lines 113–140).

**Problem:**
- After the `is_inline_name` guard and the `br` early return, the base style
  chain is `if span { .. } else if strong { .. } else { italic }`.
- `em` is reached only through the final `else`. If a new name is added to
  `Tag::is_inline_name` (`crates/rake-macros/src/view/tag.rs`), it would
  silently render italic.

**Change:** make every inline tag explicit, and make an unhandled name fail
loudly:

```rust
let base = if element.tag.is_ident("span") {
    quote!(#inherited.clone())
} else if element.tag.is_ident("strong") {
    quote!(#inherited.clone().highlight(#rake::gpui::FontWeight::BOLD.into()))
} else if element.tag.is_ident("em") {
    quote!(#inherited.clone().highlight(#rake::gpui::FontStyle::Italic.into()))
} else {
    unreachable!("Tag::is_inline_name admitted an inline tag without a style")
};
```

**Is `unreachable!` acceptable here?** Yes. The workflow guide allows panics for
documented programmer invariants, and this one is enforced a few lines above by
`is_inline_name`. It runs at compile time inside the proc macro, never at
runtime in an app.

**Add a unit test** in `crates/rake-macros/src/view/tests.rs`: expanding
`<p><span>"a"</span><strong>"b"</strong><em>"c"</em><br/></p>` succeeds. That
guards every name in `is_inline_name`. If an existing test already expands all
four inline tags, say so instead of adding a duplicate.

## Verification

Run in **bash**: the owner's shell is zsh, where `PIPESTATUS` is unavailable.

```sh
cargo test -p rake-macros --locked
cargo test -p rake --features experimental-view --test view --test view_components --test view_paragraphs --test view_ui --locked
cargo test -p rake --features view-test-support --test view_state --locked
cargo test -p rake --features experimental-view --doc --locked
cargo check -p rake-view -p renamed-facade --all-targets --locked
cargo check -p rake-view -p renamed-facade --lib --target wasm32-unknown-unknown --locked
cargo clippy -p rake-macros -p rake --features experimental-view --all-targets --locked -- -D warnings
cargo fmt --all --check
git status --short crates/rake-macros/src/view/snapshots crates/rake/tests/ui
```

**Acceptance:**
- Every command exits 0.
- The last command prints nothing, meaning snapshots and stderr are untouched.
- Don't regenerate snapshots or stderr to make tests pass.

## Commit

Commit only the cleanup work plus these nits, using explicit paths:

```sh
git add crates/rake-macros/src/view.rs crates/rake-macros/src/view/diagnostics.rs crates/rake-macros/src/view/paragraph.rs crates/rake-macros/src/view/tag.rs crates/rake-macros/src/view/tests.rs docs/experimental-view.md .agents/reports/view-macro-improvements.md
git status --short
```

Before committing, confirm in `git status --short`:
- The rename shows as `R  docs/view-macro-improvements-report.md -> .agents/reports/view-macro-improvements.md`.
- None of the owner's files (`docs/rake-inspector.md`,
  `examples/inspector/**`) or `.agents/plans/` are staged.

**Two commits** in the repo's conventional style:

1. `refactor(view): consolidate tag validation and keep emission diagnostics`
   - files: macro sources and `tests.rs`
   - body: lists cleanup tasks 1–5 and both nits
2. `docs(view): scope smoke-test claims and move process report`
   - files: `docs/experimental-view.md` and the report rename

End each commit message with:

```
Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

Don't push, and don't amend existing commits.

## Reporting

Include:
- the diff of both nits
- whether you used the inline form or the new helper for nit 1
- the new test's name, or the existing test that already covers it
- each verification command's result
- the two commit hashes, with `git show --stat` for each
- confirmation that the owner's files remain unstaged and modified
