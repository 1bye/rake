# Source linting

By default, `rake lint` checks Rust sources using stable Rust, `syn`, and the same `view!`
parser used by `rake-macros`. It does not build the checked project, run Cargo,
expand Rust macros, or require GPUI/Linux GUI libraries.

```sh
rake lint                         # current project or workspace directory
rake lint examples/view           # another project directory
rake lint --json
rake lint --json src/main.rs        # one edited file
rake lint --guide=agents          # compact AGENTS.md block
rake lint --fix --json            # safe edits, followed by a fresh check
rake lint --guide > .agents/skills/rake-gpui/SKILL.md
rake lint --explain view-button-label
rake lint --explain view-loop-index-id --json
```

The directory must contain `Cargo.toml`. Discovery recursively checks `.rs`
files, including tests, examples and `build.rs`, and nested packages. It skips
`target`, `.git`, `node_modules`, and symlinks. By default a `.rs` file with a
same-stem `.stderr` sibling is also skipped: for example, `tests/ui/bad.rs` beside
`tests/ui/bad.stderr` is a trybuild compile-fail fixture, not an application error.
Use package-relative `exclude` paths below for other intentional fixtures or
generated inputs. It does not use Cargo target
selection or `cfg` evaluation: disabled Rust branches are also checked.
Generated files outside the skipped directories are checked too. Custom target
paths outside the selected directory are not followed.

Warnings exit successfully. Any `deny` diagnostic exits with status 1. Invalid
arguments, unreadable files, malformed configuration and unknown rule IDs also
exit with status 1. Syntax errors are always denied so incomplete analysis cannot
silently succeed.

## Deep GPUI checks

`rake lint --deep` adds type-aware checks to the same syntax report. It is opt-in:
ordinary linting, `--fix`, guides and hooks retain their stable, build-free behavior.
Deep checks build the selected package (or all members at a workspace root), with
all targets and the default features for the host. They require a project directory;
use ordinary linting for single-file feedback. Dependencies, disabled features,
other target triples and unreachable macro branches are not a whole-program corpus.

```sh
rake lint --deep
rake lint examples/view --deep --json
rake lint --deep --fix       # syntax fixes, then both tiers run again
```

Install the tools explicitly before using the deep tier:

```sh
rustup toolchain install nightly-2026-08-20 --profile minimal --component rustc-dev,llvm-tools-preview,rust-src,rustfmt
cargo +nightly-2026-08-20 install --locked cargo-dylint --version 6.3.2
cargo +nightly-2026-08-20 install --locked dylint-link --version 6.3.2
```

Missing prerequisites and failed or incomplete Cargo builds exit nonzero and print
these commands. Rake never installs tools automatically. This keeps the default
workflow on stable and avoids unexpected toolchain downloads, at the cost of an
explicit setup step and a slower first deep check. Local verification also supports
installed cargo-dylint 6.2.0; CI pins 6.3.2. The library uses dylint_linting 6.2.0,
matching the existing spacing library and nightly ABI.

The CLI embeds a deterministic source archive generated from `crates/rake-cli/lints/rake-gpui/`, so an
installed binary does not need this repository. Sources are materialized in a
content-addressed directory under `CARGO_TARGET_DIR/rake-lints` (or the workspace's
`target/rake-lints`). Cargo/Dylint cache the compiled library and checked targets.
The library has its own workspace and lockfile; stable workspace builds exclude it.
The spacing library selection is unchanged.

Deep diagnostics use the existing schema version 1, catalogue IDs, Unicode columns,
ordering, package levels, discovery exclusions, human explanations and exit policy.
Warnings succeed; configured `deny` findings fail. Repeated compiler diagnostics
from library and test targets are deduplicated by source range and ID. Cargo build
errors are operational errors, not an empty successful report.

Use the same reasoned `allow(ID)` and `allow-file(ID)` comments for deep rules.
A default syntax-only run validates these directives but defers unused-file checks
for deep IDs until a deep run. The CLI forces the four compiler lints to warn so
Rust `#[allow]`, `#[deny]` and `RUSTFLAGS=-Dwarnings` cannot override the shared
catalogue policy before comment suppressions run. Configure levels in Rake metadata;
use reasoned comments for local exceptions. Raw `cargo dylint` runs retain rustc's
attribute policy.

The pinned `gpui-pre` 0.3.11 exposes the library crate `gpui`; its `Task` is
`scheduler::executor::Task` from `gpui-pre-scheduler` 0.3.11, re-exported by GPUI.
`gpui-base` 0.9.0 is a component layer, not the owner of those types. The checks
resolve compiler definitions rather than source import spelling. `smol::block_on`
resolves to `async_io::driver::block_on`, while futures uses `futures_executor`.
These rules target the pinned GPUI APIs; a future GPUI backend/API migration must
update the definition matching and UI fixtures together.

The four catalogue entries describe conservative boundaries: discarded Task
statements/wildcards, resolved Render/RenderOnce constructors, provable current or
same-local entity updates, and blocking calls directly in render or registered GPUI
listeners/action handlers. Helpers, aliases of entity identity and arbitrary call
flow are deliberately opaque. The check does not infer where nested futures and unregistered closures will run. The missing-notify rule is deferred: state mutation
alone does not prove notification is required, and a helper may already notify.

To add a deep rule:

1. Add a stable `gpui-*` catalogue entry and include it in `is_deep`; this also
   enables shared suppression, configuration and compiler-code mapping.
2. Add the late, type-aware check and lint registration to the bundled library.
   Resolve definitions and execution context; do not guess from variable names.
   Run `python3 scripts/update-deep-library.py` after changing library sources,
   configuration or its lockfile; `bundled_library_is_current` checks archive drift.
3. Add firing markers (`//~ gpui_rule_name`) and unmarked negative cases to
   `lints/rake-gpui/ui/src/lib.rs`. `python3 scripts/test-deep-lints.py` compares
   every emitted deep diagnostic to those markers using the real pinned GPUI.
4. Run `python3 scripts/check-deep-lints.py` for the workspace and every example.
   Fix findings or add a local reasoned suppression. Regenerate the catalogue
   artifacts with the commands below and run the CLI tests.

## Rules

<!-- lint-rules:begin -->
| Stable ID | Default | Check |
| --- | --- | --- |
| `view-style-class` | warn | style closure has an exact class equivalent |
| `view-theme-literal` | warn | literal color or design size bypasses the installed theme |
| `release-debug-assertions` | warn | debug assertions are explicitly enabled in a release profile |
| `view-button-label` | warn | button content has no automatic accessible label |
| `view-image-alt` | deny | image is missing alt text |
| `view-loop-index-id` | warn | loop item ID is derived from an enumerate index |
| `gpui-dropped-task` | warn | GPUI task is immediately dropped (deep) |
| `gpui-render-new` | warn | entity allocated during render (deep) |
| `gpui-reentrant-update` | warn | entity updated inside its own update (deep) |
| `gpui-ui-blocking` | warn | blocking operation on the UI thread (deep) |
| `suppression-reason` | warn | lint suppression needs a reason |
| `invalid-suppression` | warn | invalid lint suppression |
| `syntax-error` | deny | source could not be parsed |
<!-- lint-rules:end -->

The catalogue in `crates/rake-cli/src/lint/catalogue.rs` owns each rule's ID,
default level, message, explanation, fix, and before/after example. Terminal,
JSON, `--explain` and `--guide` output all use that catalogue.
The committed agent skill is `.agents/skills/rake-gpui/SKILL.md`.
The `catalogue_artifacts_are_current` test compares the entire generated skill
and this generated rule table against the catalogue; CI runs it explicitly.

The button rule matches the lowering contract: **all** children must be literal
text to receive an automatic label. Dynamic content mixed with literal text,
control flow, nested tags and icon-only/empty buttons need `aria-label`.
Use the visible text as the label (label-in-name); add context only after it.
For example, `<button aria-label={title.clone()}>{title.clone()}</button>`
preserves the visible words for speech-control users. For icon-only buttons,
use an action label, such as `<button aria-label="Search">{icon}</button>`.
A state caption such as `Backdrop: native blur` should keep that same caption in
its accessible name, rather than describe the opposite state.
Uppercase component tags own their accessibility contract and are not treated as
built-in buttons or images. Markup-valued component properties are still checked.
The check requires the attribute's presence; it cannot prove that a dynamic label
is meaningful or nonempty at runtime.

`view-image-alt` reports the same mistake that `view!` already rejects with
`img requires alt text`, without needing a build. It adds no separate image
requirement; `alt=""` remains valid for decorative images.

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
  = help: add aria-label={visible_text}; use the visible text as the label (label-in-name); add context only after it
```

The full explanation is printed in actual output. Index diagnostics underline
the ID expression; missing attributes underline the tag name.

## JSON

`--json` writes one JSON object to stdout, even when denied diagnostics cause a
nonzero exit. Operational errors use the CLI's existing stderr envelope:
`{"error":{"message":"..."}}`. Paths are relative to the selected directory,
always use `/` as the separator on every OS, and diagnostics are ordered by path,
location, then ID.

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
In `Cargo.toml`, use the same directive in a TOML `#` comment next to
`debug-assertions`; standalone directives target the next nonblank, noncomment
line. Strings (including multiline TOML strings) cannot act as directives.
Multiple directives can precede the same code line. Suppression is line scoped,
so multiple diagnostics of the same rule on that line are all suppressed.

Directives inside strings, raw strings, doc comments and block comments do not
apply. A missing reason warns and **does not suppress** the original diagnostic.
Malformed directives and unknown IDs warn. All view rules and the release-profile rule can be
suppressed inline; syntax errors and suppression diagnostics cannot hide each
other. A reason is free text, not automatically judged for adequacy.

For one deliberate decision that applies throughout a file, put a standalone
file-level directive in the header, before the first line of code:

```rust
//! Inspector views.
// rake-lint: allow-file(view-theme-literal) Inspector chrome intentionally uses fixed compact typography.

use rake::prelude::*;
```

`allow-file` accepts the same rule IDs and requires a reason, just like `allow`.
Blank lines and header comments may precede it. It suppresses every finding for
that rule in this file, including nested modules; it never applies to another
file or package. Use a line directive when the exception is local. In
`Cargo.toml`, use `# rake-lint: allow-file(ID) reason` before any TOML entries.

Missing reasons produce `suppression-reason` and do not suppress findings.
Malformed IDs, unsuppressible rules, directives after code or trailing code,
and unused file directives produce `invalid-suppression` at the directive's
source span. A file directive is used only when it suppresses a finding whose
effective level is not `allow`. Redundant duplicate file directives are unused;
remove them rather than retain stale exceptions. These diagnostics use the same
terminal/JSON output and configurable levels as invalid line suppressions.
Ordinary line directives keep their existing behavior, including no unused check.
Syntax errors and suppression diagnostics cannot be hidden by either scope.
File-suppressed findings also receive no edits from `--fix`.

## Project levels

Use the existing Rake metadata namespace in each package's `Cargo.toml`, alongside
`package.metadata.rake.bundle`:

```toml
[package.metadata.rake.lint]
exclude = ["tests/ui", "src/generated.rs"]

[package.metadata.rake.lint.levels]
view-button-label = "deny"
view-image-alt = "warn"
view-loop-index-id = "allow"
suppression-reason = "deny"
```

`exclude` accepts files or whole directory trees, relative to the package that
declares them, without globs. Empty paths, absolute paths, and `..` components are
errors. Directory names match complete path components: `tests/ui` does not
exclude `tests/ui-other`. An exclusion still applies when traversal reaches a
nested package inside that path. Exclusions do not change rule levels, and
unknown configuration keys are rejected.

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

## Exact style classes and safe fixes

`view-style-class` recognizes pure identity closures made entirely of the
zero-argument methods emitted by the shared utility compiler, such as
`|s| s.flex().gap_4()`. It rewrites the whole attribute to
`class="flex gap-4"` at the same position. It preserves the method order,
including repeated or overlapping utilities. Existing literal class attributes,
multiple style attributes, custom component properties, calls with arguments,
typed/capturing closures, statements and variants are left alone. This deliberately
misses some convertible styles rather than guess about units, types or evaluation.

`rake lint --fix` applies only advertised machine-applicable edits from emitted
diagnostics (allowed or suppressed findings are untouched). It validates source
ranges before editing, refuses stale/overlapping edits, skips files with syntax
errors, formats changed Rust files through the same engine as `rake format`,
then rechecks the project. Remaining diagnostics and the exit status describe the
edited files. Fixes are idempotent. An attribute containing comments still receives
guidance but has no automatic edit, so comments cannot be lost. Formatting errors
are reported before any prepared file is written. Checking and JSON need no build;
fixing Rust files additionally requires the installed `rustfmt`.

JSON retains schema version 1 and the human `fix` advice. Fixable diagnostics also
have `machine_fix: {"applicability":"machine-applicable","span":{...},
"replacement":"class=\"flex\""}`. Its span uses the same Unicode positions as
diagnostics and covers the entire replacement. A diagnostic-only finding omits
`machine_fix`. After `--fix --json`, fixed findings are absent.

## Theme literals

`view-theme-literal` requires an explicit `rake::theme::install(cx, theme)` call
or a `rake::App::new(...).theme(theme)` builder chain in a discovered Rust file
in the same package. Evidence crosses source files but stops at nested package
boundaries and respects discovery exclusions. It does not follow imports, renamed
facades, aliases, arbitrary helper functions or macro expansions for theme evidence.
Like the other syntax checks, it does not evaluate `cfg`.

The conservative rule diagnoses arbitrary hex colors (including variants),
arbitrary absolute typography sizes and corner radii in classes, and numeric
`rgb/rgba/hsl/hsla` or `px/rems` constructors passed to the corresponding
`bg/text_color/border_color` or `text_size/rounded` methods in pure style chains.
Constructors must be bare, `gpui::` or `rake::gpui::` paths. Ordinary layout
spacing and dimensions are left alone: they are often intentional, and the class
theme vocabulary has no general layout-size fields. Named defaults such as
`text-sm` and `rounded-md` are also left alone. Use semantic theme fields for
design tokens, or read the installed theme in a dynamic style. Intentional palette
swatches or code typography can use reasoned suppressions. There is no automatic
fix because choosing the correct semantic field requires design intent.

## Release development controls

`release-debug-assertions` checks explicit `debug-assertions = true` in
`[profile.release]`, custom profiles that transitively inherit release, and their
package/build overrides. Its fix changes only the boolean token to `false`,
preserving TOML layout and comments. Intentional diagnostics artifacts can use
`# rake-lint: allow(release-debug-assertions) <reason>` on the setting.

Feature-based release devtools detection remains deferred: Cargo profiles do not
select features. A feature declaration or dependency feature list cannot establish
the intended release invocation, command-line selection, workspace unification or
Cargo configuration overrides. This rule covers the explicit profile development
control only; it does not claim a resolved release feature graph.

## Catalogue and CI

After editing a catalogue entry, regenerate the skill with `rake lint --guide`.
Copy its generated rule table into the marked section above. Run:

```sh
cargo test -p rake-app-cli --locked --bin rake catalogue_artifacts_are_current
python3 scripts/check-example-lints.py
```

The example check runs `cargo run -p rake-app-cli --locked -- lint <example> --json`
for every top-level example package, including nested packages through discovery,
and fails on any diagnostic, including warnings. Fix findings or add a local
reasoned suppression; do not hide them with broad exclusions.

Broader loop data flow and workspace defaults remain follow-ups. Linting is not hooked into `rake dev` or DevTools.

Run [rake format](format.md) after editing, then run `rake lint`.

## Agent instructions in every session

`rake lint --guide=agents` prints a compact block generated from the rule
catalogue, between `<!-- rake-lint:begin -->` and `<!-- rake-lint:end -->`.
Copy it once into an existing `AGENTS.md`, keeping your other instructions.
Then refresh just the marked content with:

```sh
rake lint --guide=agents --update AGENTS.md
```

Refresh refuses missing, duplicated, reversed or inline markers. It preserves
everything outside them and is idempotent, including CRLF files. Use
`--guide=agents --json` for the JSON guide envelope. With `--update`, stdout is
a one-line updated/already-up-to-date confirmation; `--json` instead returns
`path` and an `updated` boolean without repeating the block. Internal parse and
invalid-directive diagnostics are left out of the compact rules; actionable
suppression guidance keeps its own line.

`rake new` writes this block to `AGENTS.md`, a complete generated skill at
`.agents/skills/rake-gpui/SKILL.md`, and a `CLAUDE.md` containing only
`@AGENTS.md`. The import syntax is documented in the official
[Claude Code memory reference](https://code.claude.com/docs/en/memory#import-additional-files).
The catalogue drift test also checks this repository's AGENTS.md block and the
scaffold templates. Edit the catalogue, regenerate the template with
`rake lint --guide=agents > crates/rake-cli/templates/default/AGENTS.md`, refresh
the root block, and regenerate the full guide before committing.

## Fast checks after an edit

```sh
rake lint --json src/main.rs
rake lint --json Cargo.toml
```

A single Rust file reports only its own findings, using its owning package's
levels, exclusions and compile-fail fixture handling. Theme evidence comes from
that package's discovered sources, stopping reads once evidence is found;
nested packages and the rest of the workspace are not scanned. A manifest input
checks only that manifest. File paths in JSON are relative to the owning package.
`--fix` also works on a single file and leaves siblings untouched. Checking uses
no Cargo build or external process; Rust fixes still use rustfmt.

On devbox, a PostToolUse Edit payload for the 2,656-line inspector view checked
through `target/debug/rake lint --hook` took **149.8 ms median** over 20 warm
invocations (147.7–169.6 ms). This used Rust 1.98.0's unoptimized dev profile
with debug info disabled, including process startup, stdin JSON, path resolution
and package theme evidence. Every invocation exited zero with empty stdout.
This is a measurement, not a CI timing assertion.

### Post-edit hooks

Set the optional PostToolUse command to `rake lint --hook`. The built-in mode
reads one JSON payload from stdin. Both tools use the same configuration below:
put it in `.claude/settings.json` for Claude Code or `.codex/hooks.json` for
local Codex sessions supporting command hooks.

```json
{
  "hooks": {
    "PostToolUse": [{
      "matcher": "Edit|Write",
      "hooks": [{
        "type": "command",
        "command": "rake lint --hook",
        "timeout": 30
      }]
    }]
  }
}
```

Claude Code treats the matcher as a regex, so `Edit|Write` also matches
`MultiEdit` and `NotebookEdit`. The hook lints any tool with a string
`tool_input.file_path`, including `MultiEdit`. `NotebookEdit` uses
`notebook_path` and is silently ignored, as are unknown tools without usable
file paths.

Use a matching `rake` executable on the agent's PATH. The formats were checked
against official documentation on 2026-11-07:

- [Claude Code hook input](https://code.claude.com/docs/en/hooks#posttooluse-input)
  supplies `tool_input.file_path` for Edit/Write;
  [hook output](https://code.claude.com/docs/en/hooks#posttooluse-decision-control)
  supports `additionalContext`.
- [Codex hooks](https://learn.chatgpt.com/docs/hooks#posttooluse) document the
  Edit/Write aliases for `apply_patch`, its `tool_input.command` payload and
  `additionalContext` output. Review and trust project hooks with `/hooks`.

The hook resolves relative paths against the payload's `cwd`, falling back to
its process directory when absent. It extracts Add/Update/Move headers from
Codex textual patches and checks surviving Rust and Cargo.toml files, skipping
deleted and non-Rust paths. Filenames are never interpolated into a shell.
Malformed JSON, a wrong event name, an unparseable Codex patch or a lint failure
produces a short manual-check note. Recognised PostToolUse events without
extractable paths emit nothing.

Clean checks emit nothing. Findings produce exactly one JSON object:

```json
{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"rake lint found 1 issue in the edited files:\nmain.rs:1:28 view-button-label (warn): button content has no automatic accessible label. Fix: add aria-label=..."}}
```

Context uses factual summaries and one compact line per finding:
`path:line:col rule-id (level): message. Fix: <fix>`. It omits long explanations
and full diagnostic JSON, shortens unusually long fields, and includes at most
20 findings followed by `N more; run rake lint`. The hook always exits zero,
including malformed/unreadable payloads or operational failures, so feedback
cannot block or undo the completed edit. It never edits files.

These are opt-in examples; this repository does not enable hooks automatically.
They match file tools, not arbitrary shell writes or other processes. Continue
running the full project check before committing. Hook tests exercise the real
binary with representative payloads; they do not configure a live agent session.
