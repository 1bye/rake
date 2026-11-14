//! Covers: offline scaffolding, CLI validation, JSON, existing directories and opt-in Git.
//! Real: CLI subprocess, embedded templates and filesystem. Doubles: none.
//! Requires: Git for the explicit --git case; no Cargo invocation or network.

use std::fs;
use std::process::{Command, Output};

fn new(directory: &std::path::Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rake"))
        .current_dir(directory)
        .arg("new")
        .args(arguments)
        .output()
        .unwrap()
}

#[test]
fn embeds_templates_and_revision_without_invoking_tools() {
    let directory = tempfile::tempdir().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rake"))
        .current_dir(directory.path())
        .env("PATH", "")
        .args(["new", "my-app", "--json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let project = directory.path().join("my-app");
    let manifest = fs::read_to_string(project.join("Cargo.toml")).unwrap();

    assert_eq!(result["name"], "my-app");
    assert_eq!(result["application_id"], "com.example.my-app");
    let revision = env!("RAKE_BUILD_REV");

    let pin = if revision.is_empty() {
        let cli: toml::Value = toml::from_str(include_str!("../Cargo.toml")).unwrap();
        let version = cli["dev-dependencies"]["rake"]["version"].as_str().unwrap();

        format!("version = \"{version}\"")
    } else {
        format!("rev = \"{revision}\"")
    };

    assert!(manifest.contains("package = \"rake-app\""));
    assert!(manifest.contains(&pin));
    assert!(!project.join("Cargo.lock").exists());
    assert!(!project.join(".git").exists());

    for file in [
        "Cargo.toml",
        "rust-toolchain.toml",
        "src/lib.rs",
        "src/main.rs",
        "web/index.html",
        ".gitignore",
        "README.md",
        "AGENTS.md",
        "CLAUDE.md",
        ".agents/skills/rake-gpui/SKILL.md",
    ] {
        let source = fs::read_to_string(project.join(file)).unwrap();

        assert!(!source.contains("{{"), "unexpanded template: {file}");
    }

    let readme = fs::read_to_string(project.join("README.md")).unwrap();

    assert!(readme.contains("rake lint --hook"));
    assert!(readme.contains("Requires Rust 1.98.0 and native development tools."));
    assert!(!readme.contains("rake-lint-hook.py"));
    assert_eq!(
        fs::read_to_string(project.join("CLAUDE.md")).unwrap(),
        "@AGENTS.md\n"
    );
    let generated = Command::new(env!("CARGO_BIN_EXE_rake"))
        .args(["lint", "--guide=agents"])
        .env("PATH", "")
        .output()
        .unwrap();

    assert!(generated.status.success());
    assert_eq!(
        fs::read(project.join("AGENTS.md")).unwrap(),
        generated.stdout
    );
    let full = Command::new(env!("CARGO_BIN_EXE_rake"))
        .args(["lint", "--guide"])
        .env("PATH", "")
        .output()
        .unwrap();

    assert_eq!(
        fs::read(project.join(".agents/skills/rake-gpui/SKILL.md")).unwrap(),
        full.stdout
    );

    assert!(
        fs::read_to_string(project.join("src/main.rs"))
            .unwrap()
            .contains("my_app::main()")
    );
}

#[test]
fn accepts_empty_directories_and_preserves_nonempty_ones() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("app");

    fs::create_dir(&project).unwrap();
    assert!(new(directory.path(), &["app"]).status.success());
    let manifest = fs::read(project.join("Cargo.toml")).unwrap();
    let refused = new(directory.path(), &["app", "--json"]);

    assert!(!refused.status.success());
    assert!(refused.stdout.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&refused.stderr).unwrap();

    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("refusing")
    );
    assert_eq!(fs::read(project.join("Cargo.toml")).unwrap(), manifest);
}

#[test]
fn rejects_bad_names_options_paths_and_revisions_without_creating_files() {
    let directory = tempfile::tempdir().unwrap();

    for arguments in [
        vec!["../escape"],
        vec!["app", "--force"],
        vec!["app", "--rake-rev", "main"],
        vec!["app", "--rake-path", "/does-not-exist"],
        vec!["app", "--rake-path", ".", "--rake-rev", "main"],
        vec!["app", "--rake-rev"],
    ] {
        let output = new(directory.path(), &arguments);

        assert!(!output.status.success(), "accepted {arguments:?}");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}

#[test]
fn supports_local_paths_explicit_revisions_and_git_without_a_commit() {
    let directory = tempfile::tempdir().unwrap();

    let rake = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../rake")
        .canonicalize()
        .unwrap();

    let output = new(
        directory.path(),
        &[
            "local",
            "--rake-path",
            rake.to_str().unwrap(),
            "--git",
            "--json",
        ],
    );

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(directory.path().join("local/.git/HEAD").is_file());
    let status = Command::new("git")
        .current_dir(directory.path().join("local"))
        .args(["rev-parse", "--verify", "HEAD"])
        .output()
        .unwrap();

    assert!(!status.status.success(), "scaffolding must not commit");
    let revision = "0123456789abcdef0123456789abcdef01234567";
    let explicit = new(directory.path(), &["pinned", "--rake-rev", revision]);

    assert!(explicit.status.success());
    assert!(
        fs::read_to_string(directory.path().join("pinned/Cargo.toml"))
            .unwrap()
            .contains(revision)
    );
}

#[cfg(unix)]
#[test]
fn refuses_symlink_destinations() {
    let directory = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();

    std::os::unix::fs::symlink(elsewhere.path(), directory.path().join("linked")).unwrap();
    assert!(!new(directory.path(), &["linked"]).status.success());
    assert_eq!(fs::read_dir(elsewhere.path()).unwrap().count(), 0);
}

#[test]
fn failed_git_init_leaves_no_project() {
    let directory = tempfile::tempdir().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rake"))
        .current_dir(directory.path())
        .env("PATH", "")
        .args(["new", "app", "--git", "--json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    assert!(serde_json::from_slice::<serde_json::Value>(&output.stderr).is_ok());
}

#[test]
fn local_dependency_paths_preserve_quotes_and_template_like_text() {
    let directory = tempfile::tempdir().unwrap();

    let name = if cfg!(windows) {
        "Rake '{{dependency}}'"
    } else {
        "Rake \"{{dependency}}\""
    };

    let checkout = directory.path().join(name);
    let rake = checkout.join("crates/rake");

    fs::create_dir_all(&rake).unwrap();
    fs::write(rake.join("Cargo.toml"), "[package]\nname = \"rake-app\"\n").unwrap();
    let output = new(
        directory.path(),
        &["app", "--rake-path", rake.to_str().unwrap()],
    );

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest = fs::read_to_string(directory.path().join("app/Cargo.toml")).unwrap();

    assert!(manifest.contains(&serde_json::to_string(&rake.canonicalize().unwrap()).unwrap()));
}

#[test]
fn generated_features_resolve_live_tools_and_the_alternate_gpui_backend() {
    let directory = tempfile::tempdir().unwrap();
    let output = new(directory.path(), &["hello"]);

    assert!(output.status.success(), "{output:?}");
    let manifest: toml::Value =
        toml::from_str(&fs::read_to_string(directory.path().join("hello/Cargo.toml")).unwrap())
            .unwrap();

    for feature in ["devtools", "live", "gpui-fast"] {
        assert_eq!(
            manifest
                .get("features")
                .and_then(|features| features.get(feature)),
            Some(&toml::Value::Array(vec![toml::Value::String(format!(
                "rake/{feature}"
            ))])),
            "generated apps must resolve {feature} without hand-editing their manifest",
        );
    }

    assert!(manifest["features"].get("default").is_none());
}
