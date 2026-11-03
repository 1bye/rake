The internal
`pest_stack_guard` helper and its dependencies are not built by `pest` without
`std`. Host-side procedural macros can still use the native stack guard when
cross-compiling a parser to another target.

## Rake patch

Vendored from the crates.io `pest_stack_guard` 1.0.0 release (MIT OR Apache-2.0).
The Rust source and build script are unchanged. The only manifest change replaces
the `psm =0.1.24` constraints with `psm ^0.1.31`. The older version archives its
Wasm object using the host archiver; on macOS this leaves unresolved
`rust_psm_*` imports and prevents GPUI browser applications from loading.
`psm` 0.1.31 writes the Wasm archive without the host archiver.

This preserves pest 2.9.5's parser validation and native stack guards. Remove the
patch when the upstream dependency constraint permits the fixed `psm` release.
Rake's committed lockfile and Rust 1.95 toolchain select the tested dependencies.
