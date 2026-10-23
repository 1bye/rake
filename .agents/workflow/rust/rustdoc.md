# Rust documentation

Use alongside [the baseline](AGENTS.md). Document contracts and decisions that a
signature cannot express. Keep examples and ownership descriptions consistent
with the implementation and its active consumers.

## Module and item documentation

- Use `//!` at the start of a crate or substantial module to explain its purpose,
  ownership, and main flow. Mention a dependency/platform boundary when meaningful.
- Use `///` immediately above public types, functions, methods, traits, errors,
  and fields whose meaning needs explanation. Keep trivial private helpers free
  of boilerplate; explain a surprising invariant or algorithm near its code.
- Begin with a concise purpose statement. Describe units, coordinate spaces,
  ordering, mutation, resource lifetime, and failure behavior when relevant.
- Distinguish owned snapshots from live views and caches. State what invalidates
  derived data, whether mutation affects another value, and whether cloning shares
  storage or copies it. Document important approximation and cost characteristics.
- Document non-finite input, clamping, empty/degenerate cases, and numeric tolerance
  when they affect a contract. Do not promise validation that is not performed.
- Use intra-doc links such as ``[`Path`]`` and ``[`Self::measure`]`` for related APIs.
  Document the public route a consumer can actually use.

```rust
/// An owned measurement snapshot of a path's full command geometry.
///
/// Coordinates and distances use node-local logical points. Curved geometry
/// is approximated by line segments. Authored trim settings are not applied.
/// Rebuild this value after changing the source geometry.
```

This is an example contract, not a mandate that all measurement APIs ignore trim.
Describe the behavior the API actually provides.

## Contract sections

Use sections when applicable, rather than adding empty headings everywhere:

- `# Examples`: a short useful consumer scenario, using public imports.
- `# Errors`: conditions that return `Err` and actionable error variants.
- `# Panics`: documented panic conditions a caller can encounter. Expected bad
  input normally belongs in `Result`, not a documented panic workaround.
- `# Safety`: obligations the caller must meet for a public unsafe operation.
  Local unsafe blocks additionally need a `// SAFETY:` explanation of why their
  preconditions hold at that call site.

Use normal prose for argument meaning rather than repeating types and parameter
names as a JSDoc-style list. Explain cancellation or blocking behavior on APIs
where it affects callers. Generated code is documented at its authoritative
schema/generator; avoid maintaining comments that regeneration overwrites.

## Executable examples

Prefer small examples that compile and run as doctests. Include assertions when
they clarify observable results. Use hidden `#` lines only for necessary setup;
keep the public usage understandable without reading hidden scaffolding.

For an error-returning example, a hidden `fn main() -> Result<..., ...>` can carry
the error context. Prefer the actual library error type when it supports standard
error propagation. Keep examples independent of secrets and live services.

Use `no_run` when compilation is useful but execution requires a runtime resource.
Use `compile_fail` to demonstrate a deliberate compile-time restriction. Use `text`
for pseudocode. `ignore` should have a concrete reason; it must not hide a broken
example. Examples involving native-only APIs need appropriate target handling.

## Verification and adoption

Run `cargo test --doc` for affected library examples and `cargo doc --no-deps`
with the package/features/target the docs describe. During documentation work,
`RUSTDOCFLAGS="-D warnings" cargo doc --no-deps` catches rustdoc warnings, including
broken intra-doc links. A successful build does not establish that prose is useful.

The lint template warns about missing public documentation. Adopt it incrementally
in an existing codebase; use narrowly explained exceptions for generated or
deliberately undocumented internals instead of suppressing it across the crate.
Do not add repetitive comments just to satisfy a warning, or equate documentation
presence with documentation quality.

References: [Rust API documentation guidelines](https://rust-lang.github.io/api-guidelines/documentation.html),
[doctests](https://doc.rust-lang.org/rustdoc/write-documentation/documentation-tests.html).
