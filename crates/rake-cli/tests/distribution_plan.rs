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

fn prebuilt_project() -> (tempfile::TempDir, String) {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("bin")).unwrap();
    std::fs::copy(
        if cfg!(unix) {
            PathBuf::from("/usr/bin/true")
        } else {
            std::env::current_exe().unwrap()
        },
        root.path().join(if cfg!(windows) {
            "bin/viewer.exe"
        } else {
            "bin/viewer"
        }),
    )
    .unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../rake-dist/tests/fixtures/prebuilt/Bundle.toml");
    std::fs::copy(fixture, root.path().join("Bundle.toml")).unwrap();
    (root, rake_dist::host_target().unwrap())
}

#[test]
fn prebuilt_discovery_plans_without_cargo_or_rustc_on_path_and_creates_nothing() {
    let (root, target) = prebuilt_project();
    let before = std::fs::read(root.path().join("Bundle.toml")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rake"))
        .current_dir(root.path())
        .env("PATH", "")
        .args(["bundle", "--plan", "--json", "--target-triple", &target])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["program"]["type"], "prebuilt");
    assert_eq!(value["target"], target);
    assert_eq!(
        value["source"]["config"]["identifier"],
        "dev.rake.prebuilt-viewer"
    );
    assert!(
        value["source"]["lock_path"]
            .as_str()
            .unwrap()
            .ends_with("Bundle.lock")
    );
    assert!(!root.path().join("dist").exists());
    assert!(!root.path().join("Bundle.lock").exists());
    assert_eq!(
        std::fs::read(root.path().join("Bundle.toml")).unwrap(),
        before
    );
}

#[test]
fn text_plan_exposes_root_lock_program_and_shadowed_metadata() {
    let (root, target) = prebuilt_project();
    std::fs::write(
        root.path().join("Cargo.toml"),
        "[package.metadata.rake.bundle]\nname='Shadowed'\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rake"))
        .current_dir(root.path())
        .env("PATH", "")
        .args(["bundle", "--plan", "--target-triple", &target])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    for label in [
        "Source:",
        "Root:",
        "Lock:",
        "Output:",
        "Program:",
        "Shadowed Cargo metadata:",
        "Native validation: pending",
    ] {
        assert!(text.contains(label), "missing {label}: {text}");
    }
}

#[test]
fn explicit_output_is_invocation_relative_while_default_is_config_root_relative() {
    let (root, target) = prebuilt_project();
    let invocation = tempfile::tempdir().unwrap();
    for explicit in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rake"));
        command
            .current_dir(invocation.path())
            .env("PATH", "")
            .args([
                "bundle",
                "--plan",
                "--json",
                "--target-triple",
                &target,
                "--config",
            ])
            .arg(root.path().join("Bundle.toml"));
        if explicit {
            command.args(["--output", "chosen"]);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let expected = if explicit {
            dunce::canonicalize(invocation.path())
                .unwrap()
                .join("chosen")
        } else {
            dunce::canonicalize(root.path()).unwrap().join("dist")
        };
        assert_eq!(PathBuf::from(value["output"].as_str().unwrap()), expected);
    }
}

#[test]
fn standalone_selectors_and_prebuilt_cargo_flags_fail_before_tools() {
    let (root, target) = prebuilt_project();
    for arguments in [
        vec!["--config", "Bundle.toml", "--manifest-path", "Cargo.toml"],
        vec!["--package", "viewer"],
        vec!["--bin", "viewer"],
        vec!["--debug"],
        vec!["--features", "foo"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rake"))
            .current_dir(root.path())
            .env("PATH", "")
            .args(["bundle", "--plan", "--json", "--target-triple", &target])
            .args(&arguments)
            .output()
            .unwrap();
        assert!(!output.status.success(), "accepted {arguments:?}");
        let value: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
        let error = value["error"]["message"].as_str().unwrap();
        assert!(
            error.contains("conflict") || error.contains("Cargo-only"),
            "{error}"
        );
    }
}

#[test]
fn explicit_manifest_keeps_metadata_fallback_and_suppresses_discovery() {
    let (root, _) = prebuilt_project();
    // An invalid discovered file must not be parsed with an explicit legacy selector.
    std::fs::write(root.path().join("Bundle.toml"), "invalid config").unwrap();
    std::fs::write(root.path().join("Cargo.toml"), "[workspace]\n[package]\nname='fallback-app'\nversion='1.0.0'\nedition='2024'\n[package.metadata.rake.bundle]\nname='Legacy Metadata'\n").unwrap();
    std::fs::create_dir(root.path().join("src")).unwrap();
    std::fs::write(root.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    assert!(
        Command::new("cargo")
            .current_dir(root.path())
            .args(["generate-lockfile", "--offline"])
            .status()
            .unwrap()
            .success()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_rake"))
        .current_dir(root.path())
        .args([
            "bundle",
            "--plan",
            "--json",
            "--manifest-path",
            "Cargo.toml",
            "--target-triple",
            "x86_64-unknown-linux-gnu",
            "--platform",
            "linux",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["plan"]["name"], "Legacy Metadata");
    assert_eq!(value["source"]["type"], "cargo_metadata");
    assert!(
        value["source"]["lock_path"]
            .as_str()
            .unwrap()
            .ends_with("rake-dist.lock")
    );
    // Explicit native target/platform flags retain the existing invocation-relative dist default.
    let output = PathBuf::from(value["output"].as_str().unwrap());
    assert_eq!(output.file_name().unwrap(), "dist");
    assert_eq!(
        std::fs::canonicalize(output.parent().unwrap()).unwrap(),
        std::fs::canonicalize(root.path()).unwrap()
    );
}

#[test]
fn legacy_schema_remains_available_without_any_tools() {
    let output = Command::new(env!("CARGO_BIN_EXE_rake"))
        .env("PATH", "")
        .args(["bundle", "--legacy-schema"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let schema: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(schema["properties"].get("program").is_none());
    assert!(schema["properties"]["sidecars"].is_object());
}

#[test]
fn worker_dispatch_revalidates_snapshot_without_reading_changed_config() {
    let (root, target) = prebuilt_project();
    let source = rake_dist::load_bundle_file(&root.path().join("Bundle.toml")).unwrap();
    let mut request = rake_dist::SourceBundleRequest::new(
        source,
        rake_dist::Platform::from_target(&target).unwrap(),
    );
    request.target = Some(target);
    request.source.config.assets = Some("../forbidden".into());
    std::fs::write(
        root.path().join("request.json"),
        serde_json::to_vec(&request).unwrap(),
    )
    .unwrap();
    std::fs::write(root.path().join("Bundle.toml"), "not valid toml anymore").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rake"))
        .env("PATH", "")
        .arg("__distribution-worker")
        .arg(root.path().join("request.json"))
        .arg(root.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(
        error.contains("confined relative path") && error.contains("forbidden"),
        "{error}"
    );
    assert!(
        !error.contains("not valid toml"),
        "worker reread the file: {error}"
    );
    assert!(!root.path().join("dist").exists());
}
