# Rust defaults

Use these defaults for Rust work. Follow the task's explicit requirements and
the target project's more specific instructions. Preserve established structure
during focused changes; apply structural migrations only within the task's scope.

## Read by task

- Read [architecture.md](architecture.md) when adding modules, changing ownership,
  designing an API, or refactoring.
- Read [testing.md](testing.md) when adding or changing behavior and its tests.
- Read [rustdoc.md](rustdoc.md) when adding or changing public contracts or docs.
- Read [stacks.md](stacks.md) for the selected runtime and target profiles, and
  [configs/README.md](configs/README.md) when adopting or changing tooling.

## Working method

- Inspect the owning module, callers, public exports, tests, Cargo manifests,
  features, and target-specific code before changing behavior.
- Keep changes with their domain owner. Reuse existing code, the standard
  library, and installed dependencies before adding an abstraction or crate.
- Keep public APIs and behavior stable during a structural refactor. Identify
  intended breaking changes explicitly and migrate affected consumers together.
- Preserve the project's edition, minimum supported Rust version (MSRV), lockfile,
  feature choices, and toolchain. An edition change is a separate migration.
- Complete the requested behavior; leave placeholders only for requested scaffolds.

## Ownership and types

- Give mutable state and resources a concrete owner and lifetime. Use `struct`
  and `impl` for cohesive state or invariants, and functions for straightforward
  transformations. A static-only service type is unnecessary for grouping functions.
- Borrow for temporary access, move when transferring ownership, and clone when
  an independent value or shared handle is actually required. Understand what a
  clone copies; `Arc::clone` and cloning a large collection have different costs.
- Prefer `&str` and slices for read-only inputs when the API does not need the
  container's ownership or capabilities. Return owned values when that simplifies
  the caller's lifetime without a demonstrated performance cost.
- Use `Arc`, `Rc`, locks, and interior mutability for real sharing requirements.
  Do not introduce them merely to silence a borrow-checker error. First examine
  the data owner, borrow duration, and operations that can be separated.
- Use enums for meaningful alternatives and newtypes when they prevent concrete
  mistakes, such as confusing coordinate spaces or units. Avoid redundant state
  that can disagree with the authoritative value.
- Prefer concrete types. Introduce traits or generics for actual substitution,
  interoperability, or reusable algorithms; choose dynamic dispatch when runtime
  selection is needed. Do not build a trait hierarchy for a single local helper.
- Validate external input at its boundary. Preserve invariants through constructors
  and methods when exposing mutable fields would let callers violate them.

## Errors and resources

- Return `Result` for expected failures and `Option` for meaningful absence. Use
  exhaustive matching instead of hiding unexpected states with a fallback.
- Give library consumers errors they can inspect, name, and propagate. Implement
  `std::error::Error` where appropriate; reuse an existing error derive dependency.
  Applications may use their established contextual error type.
- Add error context at useful boundaries and preserve causes. Log a failure at
  its handling boundary rather than repeatedly logging and propagating it.
- Reserve panic/`expect` for documented programmer invariants. Handle invalid user
  input and external failures without panic. Test setup may use clear `expect`s.
- Let resource owners clean up through `Drop` where suitable. Explicitly finish
  work whose completion can fail or must be awaited; `Drop` cannot await shutdown.
- Keep spawned work owned: define cancellation, error observation, and shutdown.
  Keep blocking operations away from an async executor's worker threads.
- Keep unsafe code confined to necessary boundaries. Explain each safety argument
  and every caller obligation; safe wrappers must enforce their own promises.

## Readability and spacing

- Use descriptive domain names and explicit units (`timeout_ms`, `width_px`).
  Keep orchestration in execution order and name intermediate values when they
  explain a stage, unit conversion, or invariant.
- Separate setup, validation, execution, and result construction with one blank
  line. Keep related short declarations together; separate multiline declarations
  and completed declaration groups from subsequent operations.
- Separate standalone `if`, `match`, loops, and `let ... else` guards from adjacent
  work. Keep `else` attached, match arms cohesive, and short guard blocks compact.
- Separate a final result expression or early return from preceding work when it
  marks a distinct result phase. Keep compact expression bodies compact; do not
  add `return` solely to satisfy a spacing convention.
- Separate type definitions, implementations, functions, and methods with one
  blank line. Keep documentation and attributes attached to their item; keep
  struct fields and enum variants cohesive.
- Keep chains and argument lists cohesive. Prefer a loop over a dense iterator
  chain when control flow or mutation is clearer. Avoid blank lines after every
  statement, artificial file-size limits, and generic `utils` dumping grounds.
- Use stable rustfmt with the project's configuration. Stable rustfmt does not
  enforce logical statement padding. Projects can adopt the shared Dylint spacing library in
  `rust/spacing` to check the conditional spacing policy automatically.

## Verification

- Run the project's existing checks for the affected package, features, and
  targets. Use the [config guide](configs/README.md) as a starting point when
  commands have not yet been established.
- Test observable behavior, meaningful failure cases, and regressions. Demonstrate
  a regression test failing before the fix when practical; do not weaken a test.
- Check externally consumed APIs through a consumer test and documentation
  examples. Internal tests alone do not establish usable public exports.
- Native checks do not establish WASM compatibility. Check supported combinations
  explicitly; `--all-targets` selects Cargo targets, not every platform triple,
  and `--all-features` can enable unsupported combinations.
- Measure performance-sensitive changes against a relevant baseline. Avoid adding
  allocation tricks, caches, or concurrency without identifying the cost and owner.
- Report checks run, their scope, and missing prerequisites. Keep source-formatting
  fixes separate from checks; compilation and tests can still write build artifacts.
