//! Covers: public view! diagnostics and offending-token source spans.
//! Real: Rust compiler, facade resolution, macro parser and expansion.
//! Doubles: none.
//! Requires: gpui and the installed Rust toolchain; no GPU execution.

#![cfg(feature = "gpui")]

#[test]
fn view_diagnostics() {
    trybuild::TestCases::new().compile_fail("tests/ui/view/*.rs");
}
