# Rake live runtime packaging

This is Blaze's `crates/blaze-live` from upstream commit
`6560553cc09f00d8d96edc24d85a5e441352ab7c`, packaged as `rake-blaze-live`.
Rake aliases it as `blaze-live` so the application code and wire protocol stay unchanged.
The original authors, repository, README and both licenses are retained. The six Rust
source files have only been reformatted with the workspace rustfmt configuration.

The upstream crate is not on crates.io. A Git-only optional dependency prevents publishing
Rake even when its live feature is disabled. This copy makes the exact supported runtime
available to registry consumers. Update it together with Rake's pinned Blaze bundle and
run the live-runtime, supervisor and live-edit acceptance checks.
