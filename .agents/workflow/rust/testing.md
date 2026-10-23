# Rust testing

Use alongside [the baseline](AGENTS.md). Keep the existing runner and useful
coverage. Apply these conventions to new or substantially changed suites.

## Placement and categories

Rust's compilation boundary and a test's behavioral category are different.
Cargo calls tests in `tests/` integration tests because they compile as external
consumers; they may still exercise only in-memory logic.

| Category | What it proves | Typical placement |
| --- | --- | --- |
| Unit | A focused rule or algorithm | Owner's `#[cfg(test)]` module |
| Component | Cooperating code in-process | Owner's tests or crate `tests/` |
| Integration | A specific real external boundary | Dedicated crate test target |
| End-to-end | A user journey through a running application | Dedicated CLI, native, or browser suite |

Small internal suites can live at the bottom of the owning module. Prefer an
adjacent `tests.rs` for a substantial suite, or `tests/mod.rs` with behavior-named
children for several cohesive suites. Declare it with `#[cfg(test)] mod tests;`.
Tests of private helpers must be descendants of the owner or use its existing
visibility; do not make the helper public just to reach it from a sibling suite.

Put external-consumer tests at `<crate>/tests/<behavior>.rs`. These tests should
import the crate as a user would and verify usable exports, error variants, and
important composed flows. Do not duplicate every internal scenario. Cargo discovers
top-level test files; nested suite files need a declared module or an explicit
`[[test]]` target. Category suffixes do not select execution automatically.

## Shared setup and assertions

- Keep one-off setup in the test. Share helpers only within the tests they serve.
- For external test targets, use `tests/common/mod.rs` and `mod common;` in each
  consumer. A top-level `tests/common.rs` becomes an extra test target.
- For internal suites, use the test parent or a private support module. Create
  a cross-crate test-support package only for real repeated consumers; depend on
  it through dev-dependencies and avoid dependency cycles.
- Factories produce fresh mutable state. Avoid shared global buffers, current
  directory changes, and process environment mutation in parallel tests. Inject
  configuration or configure a child process instead.
- Keep scenario inputs, execution, and expected results visible in the test.
  Assertion helpers are appropriate for domain comparisons, such as a geometric
  tolerance, when failures report useful actual and expected values.
- Use explicit numeric tolerances with a domain reason. For floating-point work,
  cover non-finite, degenerate, and boundary inputs when the operation accepts
  them or must reject them. Property tests supplement useful deterministic examples.
- Assert behavior and documented performance/ownership contracts. Pointer identity
  checks can be appropriate for a promised shared-allocation contract; private call
  sequences usually make poor tests.

## File summaries

Begin new or substantially changed test files before imports with a short summary:

```rust
//! Covers: parsing diagnostics and reusable measurement snapshots.
//! Real: parser, path model, and measurement algorithms.
//! Doubles: none.
//! Requires: none.
```

Name the actual external dependency and its prerequisites when present. Synthetic
inputs can still exercise a real native library or GPU. Helpers do not need these
summaries. Use a short scenario comment only when ordering or intent is non-obvious.
Separate arrangement, execution, and assertions with blank lines.

## Execution and external dependencies

Keep the default suite runnable without external services or special hardware.
Use explicit package/test targets, a non-default feature with `required-features`,
or an existing runner's selection for suites requiring a database, browser, GPU,
or native tools. Record the exact invocation and required configuration.

For example, an existing project's GPU suite can be registered explicitly:

```toml
[features]
gpu-tests = []

[[test]]
name = "gpu_render"
path = "tests/gpu_render.rs"
required-features = ["gpu-tests"]
```

Run it with `cargo test --test gpu_render --features gpu-tests` in its owning
package. An explicitly requested suite must fail clearly if prerequisites are
missing. Do not return early and report success or silently replace the dependency.
Existing ignored external suites need a documented `--ignored` invocation; avoid
ignoring regressions merely to obtain a green run.

Use isolated temporary files, ports, and resources, and clean up on failure.
Provide bounded waits and deterministic coordination instead of arbitrary sleeps.
Prefer injected clocks for time logic. Keep test credentials explicit and separate
from application/production configuration.

`cargo test` runs unit, external, and doc tests by default for selected packages.
Target selection changes that: a `--lib`, `--test`, or `--all-targets` invocation
does not establish doctest coverage. Run `cargo test --doc` for library examples
when using a targeted test command or a runner that omits doctests.

Retain the project's runner. Consider nextest, snapshot/property-test tools, or
coverage tools only for demonstrated needs. Review snapshot changes against
intended behavior; coverage percentages alone do not establish correctness.

References: [test organization](https://doc.rust-lang.org/book/ch11-03-test-organization.html),
[Cargo test selection](https://doc.rust-lang.org/cargo/commands/cargo-test.html).
