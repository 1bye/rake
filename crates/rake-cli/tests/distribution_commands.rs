//! Real portable command consumers; no signing, network or platform packaging tools.
use std::{fs, process::Command};
#[test]
fn native_build_forwards_features_and_emits_clean_json() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(root.path().join("Cargo.toml"),"[workspace]\n[package]\nname='native-build-consumer'\nversion='1.0.0'\nedition='2024'\n[features]\nsmoke=[]\n").unwrap();
    fs::write(
        root.path().join("src/main.rs"),
        "fn main(){println!(\"feature={}\",cfg!(feature=\"smoke\"));}",
    )
    .unwrap();
    assert!(
        Command::new("cargo")
            .args(["generate-lockfile", "--offline", "--manifest-path"])
            .arg(root.path().join("Cargo.toml"))
            .status()
            .unwrap()
            .success()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_rake"))
        .args([
            "build",
            "--native",
            "--features",
            "smoke",
            "--manifest-path",
        ])
        .arg(root.path().join("Cargo.toml"))
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let binary = result["artifact"].as_str().unwrap();
    let run = Command::new(binary)
        .current_dir(std::env::temp_dir())
        .output()
        .unwrap();
    assert!(run.status.success());
    assert_eq!(
        String::from_utf8(run.stdout).unwrap().trim(),
        "feature=true"
    );
}
#[test]
fn distribution_doctor_reports_cross_host_requirements_without_building() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(root.path().join("Cargo.toml"),"[workspace]\n[package]\nname='doctor-consumer'\nversion='1.0.0'\nedition='2024'\n[package.metadata.rake.bundle]\nschema_version=1\n").unwrap();
    fs::write(root.path().join("src/main.rs"), "fn main(){}").unwrap();
    assert!(
        Command::new("cargo")
            .args(["generate-lockfile", "--offline", "--manifest-path"])
            .arg(root.path().join("Cargo.toml"))
            .status()
            .unwrap()
            .success()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_rake"))
        .args([
            "doctor",
            "--distribution",
            "--platform",
            "linux",
            "--format",
            "flatpak",
            "--manifest-path",
        ])
        .arg(root.path().join("Cargo.toml"))
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["capabilities"][0]["format"], "flatpak");
    assert_eq!(result["capabilities"][0]["native_host_required"], "linux");
    assert!(!root.path().join("dist").exists());
}
