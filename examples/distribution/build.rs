//! Prepare portable desktop resources before linking.
fn main() {
    rake_build::build().expect("prepare desktop application resources");
}
