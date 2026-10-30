//! Covers: the portable CLI plan contract and read-only workspace behavior.
//! Real: CLI process, Cargo metadata, configuration validation and JSON output.
//! Doubles: none.
//! Requires: local Rust/Cargo and the existing workspace lockfile; no network.

use std::{path::PathBuf, process::Command};

#[test]
fn planning_a_cross_platform_legacy_app_preserves_the_lockfile() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let lock = root.join("Cargo.lock");
    let before = std::fs::read(&lock).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rake"))
        .current_dir(&root)
        .args([
            "bundle",
            "--plan",
            "--package",
            "rake-foundation",
            "--platform",
            "linux",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["native_validation"], "pending");
    assert_eq!(value["plan"]["platform"], "linux");
    assert_eq!(value["plan"]["packages"][0]["format"], "tar");
    assert_eq!(
        before,
        std::fs::read(lock).unwrap(),
        "planning modified the lockfile"
    );
}

#[test]
fn unsupported_arguments_produce_a_structured_failure() {
    let output = Command::new(env!("CARGO_BIN_EXE_rake"))
        .args(["bundle", "--plan", "--unknown", "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("--unknown")
    );
}

#[test]
fn schema_output_does_not_require_cargo_metadata() {
    let output = Command::new(env!("CARGO_BIN_EXE_rake"))
        .args(["bundle", "--plan", "--schema", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let schema: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(schema["properties"]["sidecars"].is_object());
    assert_eq!(schema["additionalProperties"], false);
}
