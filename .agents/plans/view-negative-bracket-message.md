# Plan: Clear errors for signed bracket values

Follow-up to `.agents/plans/view-class-grammar-fixes.md`, which was implemented as
`d794e99..8e0ae9b`.

`d794e99` made `value::number` accept only plain decimals. Signs inside brackets
are now rejected, which is intended and documented: negatives use the
utility's leading `-`. Two problems remain:

- The error is vague. `mt-[-5px]` and `w-[-5px]` both report
  "expected a finite number". Before, `w-[-5px]` said "negative values are
  supported only for margins and inset".
- Two checks in `extended_methods` (`crates/rake-macros/src/view/classes.rs`)
  can no longer trigger, because a parsed length can't be negative any more:
  - `if *n < 0. && !permits_negative { … }` (around line 193)
  - the inner `if *n < 0. { "use only one negative sign" }` inside
    `if negative { … }` (around line 200)

This is one small `fix(view)` commit. No behavior change beyond the error
messages.

## Before you start

- Read `AGENTS.md` and `.agents/workflow/rust/AGENTS.md`. Follow their spacing
  rules.
- `.agents/plans/` is untracked; leave it untracked. Stage only the files you
  change, using explicit paths.
- Don't run `cargo fmt` without `--check`. If needed, run
  `rustfmt --edition 2024 <file>` on the files you changed.

## Change

In `extended_methods`, right after the existing check:

```rust
if negative && arbitrary.is_some_and(|value| value.starts_with('-')) {
    return Err(error("use only one negative sign"));
}
```

add a check for a signed bracket value **without** the leading `-`:

```rust
if !negative && arbitrary.is_some_and(|value| value.starts_with(['-', '+'])) {
    return Err(error(if permits_negative {
        "write negative values with a leading `-`, such as `-mt-[5px]`"
    } else {
        "negative values are supported only for margins and inset"
    }));
}
```

- Use the actual prefix in the margin/inset message if it's easy, for example
  ``format!("… such as `-{prefix}-[5px]`")``. `error` currently takes
  `&'static str`, so that needs a small change. A fixed example is also fine;
  choose whichever reads more simply.
- `+` with a non-margin prefix (`w-[+5px]`) can stay under the second message
  or get "expected a finite number". Just make sure it doesn't produce the
  margin hint.
- Then delete the two dead `*n < 0.` checks. Keep `*n = -*n;` under
  `if negative`.
- `let n = match &mut length { … }` is still needed for the negation. Keep it.

## Tests

- **UI fixture** `crates/rake/tests/ui/view/negative_bracket.rs`:

  ```rust
  fn main() {
      let _ = rake::view! { <div class="mt-[-5px] w-[-5px]"/> };
  }
  ```

  Expect two errors: "1st class `mt-[-5px]`: write negative values with a
  leading `-` …" and "2nd class `w-[-5px]`: negative values are supported only
  for margins and inset". Generate the file with `TRYBUILD=overwrite`, then read
  it.
- **Keep** the existing unit test in `view/tests.rs` that `-mx-[-4px]` and
  `-mt-[-5px]` report "use only one negative sign".
- **Add** to that test, or a neighboring one, that `-mt-[5px]` still expands to
  `.mt(…px(-5f32))`.
- `negative_padding.stderr` (`-p-2`) and every other pre-existing `.stderr` file
  must be unchanged.

## Docs

`docs/experimental-view.md` already says negatives use the leading `-` and
signs are rejected. Change it only if something there now contradicts the code.

## Verification

Run in **bash**:

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
- The last command shows only the two new `negative_bracket` files.

## Commit

- Message: `fix(view): explain signed bracket values`
- End the message with:

  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  ```

- Don't push or amend.

## Reporting

Include:
- the diff
- the new `.stderr` contents
- each verification command's result
- the commit hash, with `git show --stat`
