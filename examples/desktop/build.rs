//! Prepares installed resource metadata, icons and native version information.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    rake_build::build()
}
