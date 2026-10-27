//! Covers: public view! diagnostics and offending-token source spans.
//! Real: Rust compiler, facade resolution, macro parser and expansion.
//! Doubles: none.
//! Requires: experimental-view and the installed Rust toolchain; no GPU execution.

#![cfg(feature = "experimental-view")]

#[test]
fn view_diagnostics() {
    trybuild::TestCases::new().compile_fail("tests/ui/view/*.rs");
}
