//! Covers: route derive diagnostics for ambiguous equal-specificity paths.
//! Real: the public `rake::Route` derive and compiler diagnostics.
//! Doubles: none.
//! Requires: the installed Rust toolchain; no GPU or external resources.

#[test]
fn route_diagnostics() {
    trybuild::TestCases::new().compile_fail("tests/ui/route/*.rs");
}
