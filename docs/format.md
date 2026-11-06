# Formatting Rust and view markup

Run `rake format` after editing Rust or `view!` markup:

```sh
rake format                         # cargo fmt --all, then view markup
rake fmt                            # alias
rake format src/view.rs src/app.rs   # only these files
rake format --check                 # no writes; diffs and exit 1 if changes are needed
rake format --stdin                 # one Rust file in, formatted Rust out
```

Install the matching Rake CLI and the project's pinned rustfmt. Project mode runs
from a directory containing `Cargo.toml`. It uses the same recursive source discovery
as [rake lint](lint.md), including package-relative
`[package.metadata.rake.lint].exclude`, and skips Rust fixtures with a sibling
`.stderr` file. Symlinks, `.git`, `target`, and `node_modules` are not traversed.
Explicit file arguments bypass discovery and exclusions.

Project mode runs `cargo fmt --all` first (`--check` passes through), then formats
markup. File and stdin modes run rustfmt directly first, so selecting a file
does not write its siblings. Stdin never invokes Cargo or writes source files.
Diagnostics go to stderr; stdout contains only the resulting file in stdin mode.
A malformed invocation is left unchanged and reported as `file:line:column`;
the other invocations still format, and the command exits 1. Check mode also exits
1 for formatting differences and prints unified diffs. Process/configuration
errors exit 1 with a diagnostic.

## Layout rules

Only `view!` and qualified paths ending in `view!` are formatted, following the
shared parser used by the macro and linter. Other macros such as `rsx!` retain
their bodies. Nested views inside Rust expressions and other macro arguments are
discovered too. Macro definitions are left to their author.

- Opening tags fit on one line when they have at most three attributes and fit
  the width. Otherwise attributes occupy separate indented lines, with
  `>` or `/>` on its own line.
- Existing blank lines between sibling nodes and between attributes are kept,
  with runs collapsed to one. Formatting does not introduce blank lines.
- Children are indented one level. A short text or expression child, or an
  existing short run of mixed inline content, can stay inline with its closing
  tag, such as `<span>"Saved"</span>`. If the complete line is too wide, the
  content moves below the opening tag.
- Embedded Rust uses rustfmt with the project's configuration. Short closures
  remain inline; longer expressions break within the available indentation budget.
- Markup `if`, `match`, and `for` use Rust-style braces, with indented markup
  bodies. Nested `view!` bodies use these same rules.
- The nearest ancestor `rustfmt.toml` or `.rustfmt.toml` supplies `max_width`,
  `tab_spaces`, and `hard_tabs`. Cargo editions are read from the package/workspace
  manifest; an explicit rustfmt edition takes precedence. Stdin uses the current
  directory and accepts rust-analyzer's appended `--edition EDITION` argument.

Width counts Unicode characters, rather than UTF-8 bytes, and expands tabs to
configured tab stops. Unbreakable tokens, comments, and Rust expressions that
rustfmt cannot fit can exceed the width.
There is no separate formatter configuration.

## Preservation

The formatter changes whitespace between original tokens. It never sorts
attributes, class utilities, or component properties. String literals, including
raw and multiline strings, keep every byte. Comments retain their text and their
position between neighboring tokens, including nested block comments and comments
inside Rust. Unquoted text retains the source gaps that affect its lowered text.

Rustfmt may propose syntax changes such as trailing commas. The formatter uses
its whitespace without copying those added tokens. Before writing a macro body,
it checks the original token/comment sequence and reparses the result. Formatting
runs to a fixed point; the test suite checks idempotence and token equivalence
throughout repository sources. Compiler lowering and accepted markup syntax are
unchanged.

Extracting a closure into a named binding, sorting classes, rewriting strings,
or changing markup semantics is outside this command's scope.

## rust-analyzer

Use a project-local `rust-analyzer.toml`:

```toml
[rustfmt]
overrideCommand = ["rake", "format", "--stdin"]
```

Or set the equivalent workspace editor setting, for example in VS Code:

```json
{
  "rust-analyzer.rustfmt.overrideCommand": ["rake", "format", "--stdin"]
}
```

The editor must find the matching `rake` and rustfmt executables on PATH and start
the command in the project directory. Rust-analyzer can append edition and
`--emit stdout` arguments. Format-on-save then formats both surrounding Rust and
view markup in one operation. `rake lint --guide` prints the same agent workflow:
run `rake format`, then `rake lint`.

The implementation was informed by
[leptosfmt](https://github.com/bram209/leptosfmt) (source comments and editor stdin)
and [Dioxus autofmt](https://github.com/DioxusLabs/dioxus/tree/main/packages/autofmt)
(source spans and indentation-aware width). Neither is a dependency.
