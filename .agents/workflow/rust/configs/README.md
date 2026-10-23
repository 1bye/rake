# Shared Rust tooling

Use alongside [the Rust baseline](../AGENTS.md). These templates need the
consuming project's Rust toolchain. They are references until adopted into active
configuration; files under `.agents/workflow/rust/configs/` are not automatically
loaded by Cargo or rustfmt for the project's source.

| File | Adoption |
| --- | --- |
| [rustfmt.toml](rustfmt.toml) | Copy/merge into the project or workspace root |
| [cargo-lints.toml](cargo-lints.toml) | Merge tables into the appropriate Cargo manifest |

With a Rust profile, the workflow installer's `--configs` copies `rustfmt.toml`
into the root. An existing `rustfmt.toml` or `.rustfmt.toml` is a conflict. The
installer does not edit Cargo manifests, toolchain files, scripts, or dependencies.
For mixed profiles, `--configs` also adopts the TypeScript root templates.

## Toolchain and formatting

Preserve `rust-toolchain.toml`, the edition, and `package.rust-version` when present.
For a new project, select a supported stable toolchain and edition deliberately.
Record/pin the chosen toolchain for repeatable CI; an MSRV is a compatibility floor,
not a substitute for a formatter/Clippy version. Do not pin a project to the version
that happened to be installed on the workflow author's machine.

Install missing components for the project's selected toolchain when needed:

```sh
rustup component add rustfmt clippy
```

The template uses four-space indentation, a 100-column target, and LF newlines.
It leaves edition/style-edition decisions to the project's Cargo/toolchain setup.
Align editor formatting with the same toolchain. Use rust-analyzer's Rust support
in the existing editor; a particular editor is not required by this workflow.

```sh
cargo fmt --all --check
cargo fmt --all
```

The second command changes files. Review its diff. Stable rustfmt does not provide
ESLint-style conditional padding around statements. Its configurable blank-line
bounds are unstable and do not implement that policy. For enforced conditional
spacing, adopt the shared
[Dylint spacing library](https://github.com/1bye/workflow/tree/main/rust/spacing).
It matches the Oxlint policy and pins a separate development-only nightly. The
installer does not activate it or change the application's stable toolchain.

## Compiler and Clippy lints

The fragment keeps Clippy's defaults and adds checks for leftover debug/TODO macros
and undocumented unsafe blocks. It also warns on missing public docs and denies
implicit unsafe operations within unsafe functions. These are starting policies,
not a claim that every use of those constructs is wrong.

For a single package, merge the fragment's `[lints.rust]` and `[lints.clippy]`
tables. For a workspace root, change them to `[workspace.lints.rust]` and
`[workspace.lints.clippy]`, then add this to each participating package, including
the root package if there is one:

```toml
[lints]
workspace = true
```

Cargo lint inheritance requires Rust/Cargo 1.74 or newer. Older-MSRV projects need
compatible source attributes or CI flags. Inspect actual activation before claiming
the policy applies. `clippy.toml` configures lint parameters; lint levels belong in
the manifest, source attributes, or invocation.

Start with useful warnings in an existing project and adopt strict CI after the
relevant warnings are addressed. Select additional `pedantic`/`restriction` lints
individually; do not enable the whole `restriction` group. Use narrow, explained
exceptions. Avoid crate-wide suppression or mandatory comments on trivial helpers.

## Verification commands

For a portable workspace whose default features need no special external resources:

```sh
cargo fmt --all --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

Adapt package/feature/target selection to the project. `-D warnings` also elevates
compiler warnings; pin CI's toolchain and review new warnings when upgrading it.
`--all-targets` includes binaries, tests, examples, and benchmarks for the selected
platform, not all platform triples. `--all-features` is not a universal default.

For focused work, use `-p package_name` and an existing test target/filter.
Run `cargo test -p package_name --doc` separately when the chosen runner or target
selection omitted docs. Hardware/browser/native-library suites need their recorded
prerequisites and explicit commands from [testing.md](../testing.md).

Use `--locked` in CI where a committed lockfile is part of the project's workflow.
`--offline` controls network access, not whether tests require services. Compile
checks do not replace a real host/runtime test. Dependency/security and performance
checks remain project-specific additions rather than mandatory new tool installs.

## Template verification

The repository's `bun run test:rust` exercises installed templates against temporary,
dependency-free Cargo packages/workspaces: formatting, lint activation/inheritance,
unit and consumer tests, and doctests. It requires Cargo, rustfmt, and Clippy on PATH;
missing tools fail clearly. The ordinary Bun suite tests installer behavior without
requiring a Rust toolchain. Initial validation used rustc/Cargo 1.95.0 and rustfmt 1.9.0.

References: [rustfmt options](https://github.com/rust-lang/rustfmt/blob/main/Configurations.md),
[Clippy configuration](https://doc.rust-lang.org/clippy/usage.html),
[workspace lint inheritance](https://doc.rust-lang.org/cargo/reference/workspaces.html#the-lints-table).
