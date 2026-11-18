# GPUI Pre 0.3.11

This is the Apache-2.0 crates.io release of `gpui-pre` 0.3.11 with presentation layers added.
Examples and benches are omitted from the vendored copy.

The local changes are:

- `src/presentation.rs`: spring tracks, `presentation_layer`, and the `Window` API.
- `src/scene.rs`: presentation markers in paint operations and scene composition.
- `src/window.rs`: frame-loop scheduling and presenting the composed scene.
- `src/app/test_context.rs`: a frame-request helper for tests.
- `tests/presentation.rs`: behavior coverage.

Presentation frames skip render, layout, prepaint and paint. Hit testing stays at the laid-out
position, and opacity applies per primitive rather than to a flattened group. Surfaces move but do
not fade.

Run the coverage from this directory with
`cargo test --features test-support --test presentation`.
