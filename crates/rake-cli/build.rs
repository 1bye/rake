//! Captures the exact checkout revision for offline project scaffolding.

use std::path::{Path, PathBuf};
use std::process::Command;

fn git(manifest: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .current_dir(manifest)
        .args(arguments)
        .output()
        .ok()?;

    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn checkout_revision(manifest: &Path) -> Option<String> {
    let root = git(manifest, &["rev-parse", "--show-toplevel"])?;

    let expected = Path::new(&root)
        .join("crates/rake-cli")
        .canonicalize()
        .ok()?;

    let manifest = manifest.canonicalize().ok()?;

    // An unpacked crate inside an unrelated checkout must not inherit its revision.
    if expected != manifest {
        return None;
    }

    git(&manifest, &["rev-parse", "--verify", "HEAD"])
}

fn main() {
    // Cargo treats Git source paths as immutable. A missing output watch refreshes
    // metadata even when a later Git install reuses the same build-script cache.
    if let Some(output) = std::env::var_os("OUT_DIR") {
        println!(
            "cargo:rerun-if-changed={}",
            Path::new(&output)
                .join("__capture_revision_each_build")
                .display()
        );
    }

    let revision = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .and_then(|manifest| checkout_revision(&manifest))
        .unwrap_or_default();

    // Clear inherited metadata as well when building outside a Rake checkout.
    println!("cargo:rustc-env=RAKE_BUILD_REV={revision}");
}
