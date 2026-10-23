---
name: rust-development
description: Implement, review, and refactor Rust code with explicit ownership, usable public APIs, and verification of the relevant features and targets. Use for Rust code changes and Rust-specific compiler or Clippy diagnostics.
---

# Rust development

Follow the task's scope and the project's own conventions, supported toolchain,
and target matrix. Use this skill for Rust-specific decisions; preserve established
architecture during focused fixes.

## Find the actual boundary

Read the owning module, its callers and exports, Cargo features, target-specific
dependencies, and relevant tests. Determine whether the changed code is portable
domain logic, an application operation, or a runtime/platform adapter.

For each important value, identify who owns it, who borrows it, who may mutate it,
and what ends its lifetime. For derived values, identify the source and invalidation
rule. This is particularly useful when a change appears to require a global context,
many clones, or pervasive shared mutable state.

## Choose the relevant procedure

- For borrow-checker errors, cloning, shared state, or async lifetimes, read
  [ownership.md](references/ownership.md). Fix the ownership relationship before
  widening lifetimes or wrapping data in synchronization primitives.
- For moving modules, splitting a large owner, or changing exports, read
  [refactoring.md](references/refactoring.md). Preserve behavior and test the public
  boundary while changing structure.
- For a local behavior change, identify its input/output/error contract and test
  the changed behavior through the existing owner. Do not reorganize the project
  solely to use this skill.

## Rust-specific review

- Use `Result` for expected failure and `Option` for absence. Check error context,
  cause preservation, and whether consumers can import and match public error types.
- Check contracts for empty, degenerate, and non-finite values when relevant.
  Document existing clamping/approximation rather than silently changing it during
  a structural cleanup.
- Inspect what `Clone` copies. Shared handles, copy-on-write values, and deep copies
  require different decisions; do not mechanically replace every clone with a borrow.
- Keep traits tied to a real capability boundary or generic algorithm. Consider
  concrete types and enums before adding dispatch or a parallel abstraction layer.
- Keep unsafe operations small and validate the stated safety argument against
  actual callers, aliasing, initialization, and lifetime. A comment alone is not proof.
- Verify task ownership, cancellation, and completion when async work or a worker
  is spawned. A dropped future or handle may leave external/blocking work running.

## Verify the supported use

Run focused tests and the project's formatting/Clippy checks. For exported APIs,
include an external consumer or doctest that uses public paths. Check changed
feature/platform branches explicitly; `--all-targets` is not all platforms.

Preserve the minimum supported Rust version and avoid suggestions that require a
newer language/library feature unless upgrading is part of the request. Use compiler
and installed dependency documentation to resolve uncertain APIs. Measure relevant
performance before and after changing allocation, caching, or hot algorithms.

Report the resulting ownership/API change, checks run, and any unverified target
or prerequisite. Do not claim GPU, browser, native-library, or service behavior
from a compile check or an in-memory substitute.
