//! Covers: scaffolded apps building as external consumers on native and Wasm.
//! Real: CLI, Cargo, Rake, native SDK and wasm32 compiler. Doubles: none.
//! Requires: --features scaffold-tests, Rust 1.98.0, native SDK, wasm32 target and cached/network dependencies.

use std::path::Path;
use std::process::Command;

fn run(command: &mut Command) {
    let output = command.output().unwrap();

    assert!(
        output.status.success(),
        "{command:?}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn generated_app_builds_natively_and_for_the_browser() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| workspace.join("target"));

    run(Command::new(env!("CARGO_BIN_EXE_rake"))
        .current_dir(directory.path())
        .args(["new", "consumer-app", "--rake-path"])
        .arg(workspace.join("crates/rake")));

    let project = directory.path().join("consumer-app");

    // Seed the tested workspace versions so consumer builds reuse its dependency cache.
    // Metadata adds the consumer package; all actual builds stay locked.
    std::fs::copy(workspace.join("Cargo.lock"), project.join("Cargo.lock")).unwrap();
    run(Command::new("cargo")
        .current_dir(&project)
        .args(["metadata", "--format-version", "1"]));

    for arguments in [
        vec!["build", "--locked"],
        vec![
            "build",
            "--lib",
            "--target",
            "wasm32-unknown-unknown",
            "--locked",
        ],
    ] {
        run(Command::new("cargo")
            .current_dir(&project)
            .env("CARGO_TARGET_DIR", &target)
            .args(arguments));
    }
}
