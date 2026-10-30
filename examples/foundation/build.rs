//! Prepare the application icon and native resource metadata.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    rake_build::build()
}
