//! Covers: public view! diagnostics and offending-token source spans.
//! Real: Rust compiler, facade resolution, macro parser and expansion.
//! Doubles: none.
//! Requires: gpui and the installed Rust toolchain; no GPU execution.

#![cfg(feature = "gpui")]

#[test]
fn view_diagnostics() {
    let tests = trybuild::TestCases::new();

    tests.compile_fail("tests/ui/view/*.rs");
    // Kit adds Hsla conversions, so rustc's implementation help differs.
    #[cfg(feature = "kit")]
    tests.compile_fail("tests/ui/view/kit/*.rs");
    #[cfg(not(feature = "kit"))]
    tests.compile_fail("tests/ui/view/base/*.rs");
}
