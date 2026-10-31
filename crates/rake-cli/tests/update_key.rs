//! Covers: key CLI enrollment, environment export and public-only output.
//! Real: spawned CLI, OS randomness, private files and environment references.
//! Doubles: none. Requires: local filesystem, no network or credentials.
use std::{fs, process::Command};

#[test]
fn update_key_commands_do_not_expose_or_overwrite_secrets() {
    let root = tempfile::tempdir().unwrap();

    let command = |action: &str| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rake"));

        command
            .current_dir(root.path())
            .args(["update-key", action]);

        command
    };

    let generated = command("generate")
        .args(["--private-key", "private.key", "--public-key", "public.key"])
        .output()
        .unwrap();

    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let secret = fs::read_to_string(root.path().join("private.key")).unwrap();
    let stdout = String::from_utf8(generated.stdout).unwrap();

    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&stdout).unwrap()["public_key"],
        "public.key"
    );
    assert!(!stdout.contains(secret.trim()));
    let exported = command("public")
        .args(["--env", "RAKE_TEST_SEED", "--public-key", "from-env.pub"])
        .env("RAKE_TEST_SEED", secret.trim())
        .output()
        .unwrap();

    assert!(
        exported.status.success(),
        "{}",
        String::from_utf8_lossy(&exported.stderr)
    );
    assert_eq!(
        fs::read(root.path().join("from-env.pub")).unwrap(),
        fs::read(root.path().join("public.key")).unwrap()
    );
    let repeated = command("generate")
        .args([
            "--private-key",
            "private.key",
            "--public-key",
            "another.pub",
        ])
        .output()
        .unwrap();

    assert!(!repeated.status.success());
    assert_eq!(
        fs::read_to_string(root.path().join("private.key")).unwrap(),
        secret
    );
    assert!(!root.path().join("another.pub").exists());
    let invalid = command("public")
        .args(["--env", "RAKE_TEST_SEED", "--public-key", "invalid.pub"])
        .env("RAKE_TEST_SEED", "not-a-key")
        .output()
        .unwrap();

    assert!(!invalid.status.success());
    assert!(!String::from_utf8_lossy(&invalid.stderr).contains("not-a-key"));
}
