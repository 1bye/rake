//! Captures the exact checkout revision for offline project scaffolding.

use std::path::Path;
use std::process::Command;

fn git(arguments: &[&str]) -> Option<String> {
    let output = Command::new("git").args(arguments).output().ok()?;

    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    // Worktrees keep HEAD and branch refs in different Git directories.
    for name in ["HEAD", "packed-refs"] {
        if let Some(path) = git(&["rev-parse", "--git-path", name]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }

    if let Some(reference) = git(&["symbolic-ref", "-q", "HEAD"])
        && let Some(path) = git(&["rev-parse", "--git-path", &reference])
    {
        println!("cargo:rerun-if-changed={path}");
    }

    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));

    // An unpacked crate inside an unrelated checkout must not inherit its revision.
    if let Some(root) = git(&["rev-parse", "--show-toplevel"])
        && Path::new(&root).join("crates/rake-cli") == manifest
        && let Some(revision) = git(&["rev-parse", "--verify", "HEAD"])
    {
        println!("cargo:rustc-env=RAKE_BUILD_REV={revision}");
    }
}
