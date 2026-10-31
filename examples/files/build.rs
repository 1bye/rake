//! Validates distribution metadata and embeds installed policy.
fn main() {
    rake_build::build().expect("Text Toolkit distribution metadata must be valid");
}
