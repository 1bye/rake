# Source linting

`rake lint` checks Rust sources using stable Rust, `syn`, and the same `view!`
parser used by `rake-macros`. It does not build the checked project, run Cargo,
expand Rust macros, or require GPUI/Linux GUI libraries.

```sh
rake lint                         # current project or workspace directory
rake lint examples/view           # another project directory
rake lint --json
rake lint --explain view-button-label
rake lint --explain view-loop-index-id --json
```

The directory must contain `Cargo.toml`. Discovery recursively checks `.rs`
files, including tests, examples and `build.rs`, and nested packages. It skips
`target`, `.git`, `node_modules`, and symlinks. It does not use Cargo target
selection or `cfg` evaluation: disabled Rust branches are also checked.
Generated files outside the skipped directories are checked too. Custom target
paths outside the selected directory are not followed.

Warnings exit successfully. Any `deny` diagnostic exits with status 1. Invalid
arguments, unreadable files, malformed configuration and unknown rule IDs also
exit with status 1. Syntax errors are always denied so incomplete analysis cannot
silently succeed.

## Rules

| Stable ID | Default | Check |
| --- | --- | --- |
| `view-button-label` | warn | A built-in button without `aria-label` cannot derive a nonempty literal-only name. |
| `view-image-alt` | deny | A built-in `img` has no `alt`; `alt=""` is valid for decorative images. |
| `view-loop-index-id` | warn | An `id` is visibly derived from an `.enumerate()` index. |
| `suppression-reason` | warn | A suppression has no reason and is ignored. |
| `invalid-suppression` | warn | A directive is malformed or names an unknown/unsuppressible rule. |
| `syntax-error` | deny (fixed) | Rust or view markup could not be parsed. |

The catalogue in `crates/rake-cli/src/lint/catalogue.rs` owns each rule's ID,
default level, message, explanation, fix, and before/after example. Terminal,
JSON and `--explain` output all use that catalogue.

The button rule matches the lowering contract: **all** children must be literal
text to receive an automatic label. Dynamic content mixed with literal text,
control flow, nested tags and icon-only/empty buttons need `aria-label`.
Uppercase component tags own their accessibility contract and are not treated as
built-in buttons or images. Markup-valued component properties are still checked.
The check requires the attribute's presence; it cannot prove that a dynamic label
is meaningful or nonempty at runtime.

The loop rule follows the first binding in `(index, item)` from `.enumerate()`
inside markup `for` nodes;
`rev`, `skip`, `take`, `step_by`, and `by_ref` preserve that relationship. It checks
paths, tuples, casts, arithmetic, methods on the index, and `format!` arguments
or captured names. Data bindings named `i`, fields named `index`, indexed data
keys such as `rows[index].id`, and bindings shadowed by nested loops or patterns
are not mistaken for positional IDs. Other adapters, arbitrary calls, blocks,
integer ranges, enclosing Rust loops, aliases and data flow are opaque. Prefer missing an uncertain
case over diagnosing a data key as an index.

```rust
// Before
rake::view! {
    <div>for (index, row) in rows.iter().enumerate() {
        <button id={ ("row", index) }>{row.title.clone()}</button>
    }</div>
}
// After
rake::view! {
    <div>for row in &rows {
        <button id={ ("row", row.id) } aria-label={row.title.clone()}>
            {row.title.clone()}
        </button>
    }</div>
}
```

This is parsing and targeted linting, not the macro's full semantic validation.
Class utilities, attribute types, builder calls and Rust macro expansions still
belong to the compiler. View invocations in ordinary syntax and nested macro
arguments are checked, but `macro_rules!` definitions are left opaque.

## Terminal diagnostics

Diagnostic text goes to stderr; the summary goes to stdout.

```text
warning[view-button-label]: button content has no automatic accessible label
 --> src/main.rs:4:14
  |
  4 |             <button>{icon}</button>
  |              ^^^^^^
  = note: Rake automatically labels only nonempty buttons made entirely of literal text. ...
  = help: add aria-label="Describe the action" to the button
```

The full explanation is printed in actual output. Index diagnostics underline
the ID expression; missing attributes underline the tag name.

## JSON

`--json` writes one JSON object to stdout, even when denied diagnostics cause a
nonzero exit. Operational errors use the CLI's existing stderr envelope:
`{"error":{"message":"..."}}`. Paths are relative to the selected directory,
use the host's separators, and diagnostics are ordered by path, location, then ID.

```json
{
  "schema_version": 1,
  "diagnostics": [{
    "rule_id": "view-image-alt",
    "level": "deny",
    "file": "src/main.rs",
    "span": {
      "start": {"line": 1, "column": 28},
      "end": {"line": 1, "column": 31}
    },
    "message": "image is missing alt text",
    "explanation": "The img tag maps alt to its accessible label. Describe meaningful images; an empty alt explicitly marks a decorative image. aria-label does not replace the required alt attribute.",
    "fix": "add alt=\"Description\", or alt=\"\" for a decorative image"
  }],
  "files_checked": 1,
  "denied": true
}
```

Locations use **1-based Unicode character columns and lines**, with an exclusive
end position. They refer to the original file, not regenerated token text.
Allowed and suppressed diagnostics are omitted. `denied` is true if any emitted
diagnostic has level `deny`. `--explain ID --json` returns `schema_version` and a
`rule` object containing `id`, `default_level`, `message`, `explanation`, `fix`,
`before`, and `after`.

## Suppression

Use a real Rust line comment with exactly one rule ID and a nonempty reason:

```rust
rake::view! {
    <div>
        // rake-lint: allow(view-button-label) The mounted control supplies its own accessible name.
        <button>{named_control}</button>
    </div>
}
```

A standalone directive applies to the next nonblank line that is not a line
comment. A trailing directive applies to its own line. Put the directive next
to the opening tag (or the ID expression's starting line for the index rule).
Multiple directives can precede the same code line. Suppression is line scoped,
so multiple diagnostics of the same rule on that line are all suppressed.

Directives inside strings, raw strings, doc comments and block comments do not
apply. A missing reason warns and **does not suppress** the original diagnostic.
Malformed directives and unknown IDs warn. Only the three view rules can be
suppressed inline; syntax errors and suppression diagnostics cannot hide each
other. A reason is free text, not automatically judged for adequacy.

## Project levels

Use the existing Rake metadata namespace in each package's `Cargo.toml`, alongside
`package.metadata.rake.bundle`:

```toml
[package.metadata.rake.lint.levels]
view-button-label = "deny"
view-image-alt = "warn"
view-loop-index-id = "allow"
suppression-reason = "deny"
```

The accepted levels are `allow`, `warn`, and `deny`. Unknown keys and levels are
errors. `syntax-error` is always denied and cannot be overridden. The nearest
ancestor package manifest supplies levels for its files. Nested packages start
with their own settings and catalogue defaults; levels are not inherited across
package boundaries. Workspace-wide defaults are deferred; set levels per member.
A Cargo workspace root without package metadata uses catalogue defaults.

## Adding a rule

1. Add a stable ID and complete entry in `catalogue.rs`. Explain the problem,
   consequence and exact fix, with before/after examples. Start uncertain rules
   as warnings and narrow them until they avoid false positives.
2. Add the syntax check in `source.rs`, using `rake-view-syntax` nodes and original
   spans. Add its ID to the suppressible set when appropriate. Do not add another
   view parser or type guesses based on identifier names.
3. Add firing/non-firing and span tests in `lint/tests.rs`, and binary tests in
   `crates/rake-cli/tests/lint.rs` for changes to the command contract. Review JSON
   snapshot changes and run the linter across every example crate.
4. Document the rule and limitations here. Keep compiler and expansion snapshots
   unchanged unless a separate task explicitly changes macro semantics.

## Deferred work

A manifest-only release/devtools rule is not shipped. `rake` wires `devtools`
through dependency features (`devtools-memory` enables it too); control and
inspection paths use feature gates rather than Cargo profiles. Cargo profiles do
not select features, and manifest inspection cannot resolve command-line feature
selection, workspace feature unification, aliases or release scripts. Checking
an ordinary feature declaration would incorrectly label development-only use as
a release mistake. A future release-aware check needs a defined build invocation
or a resolved feature graph, with tests for actual release selection.

The deep/type-aware tier, `--fix`, `--guide`/skill generation, style-closure and
hard-coded-colour rules, broader loop data flow and workspace defaults remain
follow-ups. Linting is not hooked into `rake dev`, DevTools or CI.
