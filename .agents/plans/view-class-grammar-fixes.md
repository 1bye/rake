# Plan: `rake::view!` class grammar fixes

Follow-up to `.agents/plans/view-class-grammar-and-themes.md`, which was implemented as
commits `eedc44a..7d534c7`. A review found one real bug (task 1) and five small
usability problems (tasks 2–6). Every task is independent. Use one commit per
task, or one `fix(view)` commit for task 1 and one for tasks 2–6.

## Before you start

- Read `AGENTS.md` and `.agents/workflow/rust/AGENTS.md`. Follow their spacing
  and verification rules.
- Run `git status`. `.agents/plans/` is untracked; leave it untracked. Stage
  only the files you change, using explicit paths.
- Code lives here:
  - `crates/rake-macros/src/view/classes.rs`: `LENGTH_PREFIXES`, `extended_methods`, `utility_methods`, `FIXED`
  - `crates/rake-macros/src/view/classes/value.rs`: `number`, `scale`, `length`, and their unit tests
  - `crates/rake-macros/src/view/classes/branch.rs`: `Branches::select`, `check_pattern`, `compile_with`
  - `crates/rake-macros/src/view/classes/error.rs`: hints
  - `crates/rake-macros/src/view.rs`: `parse_class_name` (around line 427)
  - UI fixtures: `crates/rake/tests/ui/view/*.rs` with `.stderr`
  - Guide: `docs/experimental-view.md`, including the "Class grammar" table
    around line 102 and the utility list between `<!-- view-utilities:start/end -->`
    (a test reads that list)
- Don't run `cargo fmt` without `--check`. If formatting is needed, run
  `rustfmt --edition 2024 <file>` on the files you changed.

## Snapshot and stderr policy

- Expansion snapshots (`crates/rake-macros/src/view/snapshots/`) must stay
  byte-identical, unless a task below names a change.
- New `.stderr` fixtures may be generated with `TRYBUILD=overwrite`.
- Read every new or changed `.stderr` before keeping it.
- Report each changed pre-existing `.stderr` file with a one-line reason.

---

## Task 1 (bug): Number-only `border-`, `text-` and `rounded-` classes

**Problem:**
- `border`, `border-t/b/l/r`, `rounded` and `text` are in `LENGTH_PREFIXES`.
- A number that `FIXED` doesn't list falls through to `value::scale`, which
  uses steps of 0.25rem.
- `border-0/1/2` are in `FIXED` and map to GPUI's `border_N`, which is N px.

So the results are inconsistent:

| Class | Today | Expected |
| --- | --- | --- |
| `border-2` | 2px | 2px |
| `border-3` | 12px (`rems(0.75)`) | 3px |
| `border-t-4` | 16px | 4px |
| `border-1.5` | 6px | 1.5px |
| `text-13` | 52px font size | compile error |
| `rounded-4` | 1rem radius | compile error |

GPUI's own `border_N` methods are all in px (0–12 and 16; see
`gpui_macros/src/styles.rs` `border_suffixes` in the pinned checkout
`~/.cargo/git/checkouts/gpui-ce-866ba02453e968cb/99109e3`). Tailwind also uses
px for `border-N`, and has no numeric `text-N` or `rounded-N`.

**Change, in `extended_methods`, after `raw` is known and before scale parsing:**
- If `prefix` is `border` or `border-{t,b,l,r}`, and `raw` is a plain number
  (not bracketed, not a fraction):
  - parse it with the stricter number check from task 5
  - require a value of 0 or more
  - emit `px(N)` through `length_tokens(value::Length::Pixels(n), rake)`
- If `prefix` is `text` or `rounded`, and `raw` is a plain number, return
  errors:
  - `text-N`: "text sizes use a name (`text-sm`), a theme field or `[11px]`; numeric text-N isn't supported"
  - `rounded-N`: "radius uses a name (`rounded-md`), a theme field or `[4px]`"
- Keep `FIXED` `border-0/1/2` as they are. They already produce px, and
  snapshots depend on them.
- Fractions and `%` stay rejected for these prefixes, as today.

**Check theme validation:**
- `reserved` in `crates/rake-macros/src/view/theme.rs` already reserves `t_N`,
  `b_N`, `l_N` and `r_N`. Confirm it still matches, and that
  `theme_reserved_scale` is unchanged.

**Tests:**
- Unit test in `view/tests.rs`: `border-3` expands to `border(…px(3f32))`,
  `border-t-1.5` expands to `border_t(…px(1.5f32))`.
- Frame or style comparison, next to the existing arbitrary-value style tests in
  `crates/rake/tests/view.rs`: `border-3` equals `border-[3px]`.
- New UI fixtures `text_numeric.rs` and `rounded_numeric.rs`.

**Docs:**
- In the "Class grammar" table, the `prefix-N` row says each step is 0.25rem.
  Add that `border-N` is N px, and that `text-` and `rounded-` take names or
  `[value]`.
- Update any example that uses `text-N` or `rounded-N` with a number. Search
  for them with `git grep -nE '"[^"]*\b(text|rounded)-[0-9]'`, and include
  `examples/`.

---

## Task 2: Allow bare unit variants such as `None` in class patterns

**Problem:**
- `check_pattern` in `branch.rs` rejects every `Pat::Ident`, so
  `match o { Some(_) => "p-1", None => "p-2" }` fails.
- Users must write `Option::None`.

**Change:**
- Accept a `Pat::Ident` when all of these hold:
  - no `by_ref`
  - no `mutability`
  - no `subpat` (`@`)
  - the identifier starts with an ASCII uppercase letter
- Uppercase names are unit variants or constants by Rust convention. If one
  were a binding, rustc already warns about it (`non_snake_case`), and
  `matches!` would just accept everything.
- Keep rejecting lowercase identifiers. Update the error message to:
  "class patterns cannot bind variables; use `_`, a constant or unit variant (`None`, `Mode::Dark`), or a plain comparison".
- Update the `lib.rs` macro doc sentence "Qualify unit variants/constants (for
  example Option::None)", and the same sentence in the guide.

**Tests:**
- Add a runtime test in `crates/rake/tests/view_state/classes.rs` or
  `tests/view.rs`: a `match` with `Some(_)` / `None` picks the right leaf.
- `class_binding_pattern.stderr` changes only in its message text. Report it.

---

## Task 3: Reject let chains in class conditions with a clear error

**Problem:**
- `class={if let Some(_) = o && b { "p-1" } else { "p-2" }}` produces rustc's
  "expected expression, found `let` statement".
- `select` only handles a condition that is exactly `Expr::Let`. A chain is
  `Expr::Binary(&&)` containing a `let`, and is emitted as `let c: bool = <chain>;`.

**Change, choose one and say which:**
- **(Preferred)** Support the chain. Lower `if A && let P = e && B` into nested
  checks that run left to right and stop early, with each `let` becoming
  `matches!(&(e), P)` and its pattern checked by `check_pattern`. The result
  is bound to a single `bool` local, as today.
- **Alternatively**, detect any `Expr::Let` inside the binary tree and return
  "let chains aren't supported in class conditions; compute a bool first".

Either way, add a fixture or test that pins the behavior. If you support
chains, add a runtime test showing that `B` isn't evaluated when the `let`
fails.

---

## Task 4: Allow a negative utility after a namespaced variant

**Problem:**
- `class:hover:-mt-1={b}` fails with "expected ident", plus a misleading second
  error, "unsupported attribute `mt-1`".
- `parse_class_name` in `view.rs` accepts `-` only before the first segment.

**Change:**
- In the loop, after consuming a `:` punctuation, also accept an optional
  leading `-` before the next segment, the same way the start does.
- Don't accept `-` after `/` or `-`.

**Tests:**
- Unit or runtime test: `class:hover:-mt-1={b}` expands to a hover refinement
  with a negative margin.
- Also cover `class:-mt-1={b}`, which already works.

---

## Task 5: Accept only plain decimal numbers in scale values

**Problem:**
- `value::number` uses `f32::from_str`, so `p-1e1` (10 steps) and `w-4.` are
  accepted.
- The same parser is used for `[..]` values and opacity.

**Change:**
- Add a strict check used by `scale`, `length`, opacity brackets and task 1:
  digits, optionally followed by `.` and one or more digits.
- No sign. Negatives are handled by the leading `-` prefix and the existing
  checks.
- No exponent, no leading or trailing `.`, no `inf`/`NaN`.
- Keep the existing error messages.

**Tests:** extend
`values_require_finite_numbers_units_and_valid_ranges` in `value.rs` with these
inputs, which must fail: `"1e1"`, `"4."`, `".5"`, `"+4"`, `"1e1px"`.

Check that `-mt-[-5px]` still gives "use only one negative sign". If the strict
parser now rejects `-5px` first, keep a specific message for it, and update
that fixture only if its message changes. Report it if so.

---

## Task 6: Explain group conflicts between class branches

**Problem:**
- In `class={if b { "group-hover/a:p-1" } else { "group-hover/b:p-2" }}`, only
  one leaf can apply at a time.
- It is still rejected with "GPUI supports one named group-hover group per
  element".
- The rejection is correct: the plan emits one `.group_hover(name, …)` call per
  name, and two calls trip GPUI's debug assertion. Only the message is the
  problem.

**Change:**
- `compile_with` in `branch.rs` calls `plan.extend(leaf, value.span())`.
  When that returns the group error, replace the message with: "GPUI
  supports one named {kind} group per element, across all class branches; use
  the same group name in every branch".
- Leave `StylePlan::extend`'s own message as it is for the non-branch case.

**Test:** new UI fixture `class_branch_group_conflict.rs`.

---

## Out of scope (don't do these)

- Detecting conflicts between a preset's variants and the element's classes.
  It's documented as a known limitation.
- A theme fallback when `rake::theme::current()` finds nothing installed.
- Changing `FIXED` entries, theme runtime APIs or the inspector example.

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
git status --short crates/rake-macros/src/view/snapshots
```

**Acceptance:**
- Every command exits 0.
- The last command prints nothing.
- Only the `.stderr` files named in the tasks changed.

## Commit

- Conventional style, for example:
  - `fix(view): use px for numeric border widths`
  - `fix(view): accept unit variants and let chains in class branches`
- End each message with:

  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  ```

- Don't push or amend existing commits.

## Reporting

Include:
- each task's diff summary and files
- for task 3, which option you chose
- every new and changed `.stderr` file, with a one-line reason
- each verification command's result
- the commit hashes, with `git show --stat` for each
