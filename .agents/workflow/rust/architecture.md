# Rust architecture

Use alongside [the baseline](AGENTS.md). These are defaults for new boundaries,
not a requirement to reorganize an existing project during unrelated work.

## Organize by domain

Group by the concept that owns the behavior: `path`, `media`, `trace`, `tracking`,
or `report`. Within that boundary, split by a responsibility a reader can name.
Keep types, constants, errors, and helpers with their owner until a separate file
has a useful purpose. Do not begin with global `services/`, `models/`, and `utils/`
directories that scatter one feature across the repository.

```text
src/
  lib.rs                 # Intended crate API
  main.rs                # CLI/runtime setup, when needed
  path/
    mod.rs               # Purpose, module declarations, public exports
    model.rs             # Path representation and core construction
    parser.rs            # Parsing and parser-owned conversion helpers
    measurement.rs       # Measurement data and queries
    errors.rs            # Related public failure contracts
    tests/
      mod.rs             # Register suites; small local shared helpers
      parser.rs
      measurement.rs
tests/
  path_api.rs            # External-consumer contract tests
```

Create only the roles the module needs. A small concept fits in one file.
Both `path.rs` with a `path/` directory and `path/mod.rs` are valid Rust layouts;
preserve the existing choice. For new multi-file domain modules, prefer `mod.rs`
as the local entry point. File length prompts a review of responsibilities, not
an automatic split.

## Public boundaries

- Keep implementation modules private by default. Use explicit `pub use` exports
  at meaningful domain/crate boundaries so consumers can use a coherent API.
- Expose the error, options, and result types consumers need to name. A public
  method returning a type hidden behind a private module is an incomplete API.
- Use the narrowest useful visibility: private, `pub(super)`, a scoped `pub(in ...)`,
  `pub(crate)`, then public. Do not widen production visibility just for tests.
- Prefer explicit imports in production modules. Avoid a parent module collecting
  dependencies solely so every child can use `super::*`. A small test module can
  use `super::*` when its parent is the obvious subject under test.
- Public fields suit plain data with no protected invariants. Use private fields
  and focused methods when ownership, validation, or state transitions matter.
- Separate wire/input representations from domain representations only when
  validation, compatibility, or ownership makes them meaningfully different.

## Types and behavior

A `struct` with `impl` owns meaningful data, dependencies, invariants, or resource
lifetime. A module groups related free functions. Methods may be distributed
across responsibility-specific `impl` blocks for the same local type; keep the
type's definition and core construction discoverable.

Start with concrete dependencies passed as arguments or stored by a real owner.
Introduce a trait when implementations vary, a consumer requires a capability,
or a real external dependency needs substitution in tests. Do not wrap every
operation in a service, repository, factory, and interface.

Use enums when alternatives are known and exhaustive handling helps. Use a trait
when the set of implementations should be open. A trait need not become `dyn`:
choose generics or dynamic dispatch according to how callers select behavior.

## Processing and resource ownership

For data processing, make stages visible: input decoding, validation, domain
work, and output conversion. Keep calculations separate from file/network/GPU
operations where doing so makes dependencies and tests simpler. Stateful engines
can still have pure calculations inside them.

An orchestrator should show the order and dependencies of those stages. Split a
large state owner when a subsystem has its own invariants or lifecycle, rather
than merely distributing methods across files that all mutate the same fields.
Pass a subsystem the data/resources it needs when practical; avoid universal
context objects and repeated forwarding wrappers with no behavior.

Distinguish operation-local state, long-lived resources, and derived snapshots.
For example, a parser owns its cursor for one parse; a path owns commands; a
measurement owns derived data for repeated queries. Document when derived data
must be rebuilt. A cache additionally needs a key, invalidation or replacement
policy, capacity/eviction behavior, and an explicit owner.

Keep helpers with their real consumers. A parser-only curve conversion belongs
to parsing even if it produces geometry used elsewhere. Move a helper to a shared
owner once multiple consumers share the same contract, not because reuse seems
possible. Keep changes to approximation, allocation, and numeric edge behavior
separate from mechanical moves when possible.

## Modules, crates, and platforms

Use a module for ordinary organization. Add a crate for a meaningful dependency,
platform, reusable API, or build boundary. A crate per small feature adds Cargo
and public API overhead without automatically improving ownership.

Keep runtime adapters dependent on the domain they expose. Domain code should
not import CLI parsing, JS bindings, or GPU initialization to represent its data.
Target-specific dependencies and `cfg` branches belong near the platform boundary;
avoid leaking platform types throughout portable code.

Document each crate's responsibility and actual dependency direction. Workspace
membership alone does not enforce layering: use crate dependencies and module
privacy, then review unexpected imports. Keep generated contracts attached to
their generator and verify their real consumers.

## Refactoring sequence

1. Identify current entry points, callers, exports, ownership, and observable
   behavior. Include enabled features and supported platforms.
2. Run the relevant baseline checks. Add a missing contract/regression test where
   it protects the behavior being moved, rather than duplicating existing suites.
3. Move one cohesive responsibility and update its callers/imports. Preserve
   externally consumed paths through deliberate reexports when compatibility matters.
4. Verify the affected behavior and consumer boundary. Benchmark if the change
   alters hot-path work, allocations, caching, or concurrency.
5. Update module documentation to describe the resulting ownership and flow.

Reference: [Rust modules and privacy](https://doc.rust-lang.org/book/ch07-02-defining-modules-to-control-scope-and-privacy.html).
