//! Covers: lint CLI JSON, explanations, argument errors, suppression and exit status.
//! Real: rake binary, source discovery, configuration, and source parser.
//! Doubles: temporary project sources; Cargo is deliberately unavailable to the child.
//! Requires: no display, GPU, network, or external binaries.

use std::fs;
use std::process::{Command, Output};

fn run(source: &str, config: &str, arguments: &[&str]) -> Output {
    let project = tempfile::tempdir().unwrap();

    fs::write(
        project.path().join("Cargo.toml"),
        format!("[package]\nname='lint-fixture'\nversion='0.1.0'\n{config}"),
    )
    .unwrap();
    fs::write(project.path().join("main.rs"), source).unwrap();

    Command::new(env!("CARGO_BIN_EXE_rake"))
        .arg("lint")
        .args(arguments)
        .current_dir(project.path())
        .env("PATH", "")
        .output()
        .unwrap()
}

#[test]
fn json_exit_status_distinguishes_warn_deny_and_allow() {
    let warning = run("fn main() { rake::view! { <button/> }; }", "", &["--json"]);
    let denied = run(
        "fn main() { rake::view! { <img src=\"a\"/> }; }",
        "",
        &["--json"],
    );
    let allowed = run(
        "fn main() { rake::view! { <img src=\"a\"/> }; }",
        "[package.metadata.rake.lint.levels]\nview-image-alt='allow'",
        &["--json"],
    );

    assert!(
        warning.status.success(),
        "{}",
        String::from_utf8_lossy(&warning.stderr)
    );
    assert!(warning.stderr.is_empty());
    assert_eq!(denied.status.code(), Some(1));
    assert!(denied.stderr.is_empty());
    assert!(allowed.status.success());
    let warning: serde_json::Value = serde_json::from_slice(&warning.stdout).unwrap();
    let denied: serde_json::Value = serde_json::from_slice(&denied.stdout).unwrap();
    let allowed: serde_json::Value = serde_json::from_slice(&allowed.stdout).unwrap();

    assert_eq!(warning["schema_version"], 1);
    assert_eq!(warning["denied"], false);
    assert_eq!(warning["diagnostics"][0]["rule_id"], "view-button-label");
    assert_eq!(warning["diagnostics"][0]["level"], "warn");
    assert_eq!(denied["denied"], true);
    assert_eq!(allowed["diagnostics"], serde_json::json!([]));
}

#[test]
fn deny_override_and_reasoned_suppression_control_exits() {
    let config = "[package.metadata.rake.lint.levels]\nview-button-label='deny'";
    let source = "fn main() { rake::view! { <button/> }; }";
    let denied = run(source, config, &[]);
    let suppressed = run(
        &format!("// rake-lint: allow(view-button-label) Embedded widget names itself.\n{source}"),
        config,
        &[],
    );
    let missing_reason = run(
        &format!("// rake-lint: allow(view-button-label)\n{source}"),
        config,
        &[],
    );

    assert_eq!(denied.status.code(), Some(1));
    let terminal = String::from_utf8(denied.stderr).unwrap();

    assert!(terminal.contains("error[view-button-label]:"));
    assert!(terminal.contains("--> main.rs:1:28"));
    assert!(terminal.contains("^^^^^^"));
    assert!(terminal.contains("= help: add aria-label="));
    assert!(suppressed.status.success());
    assert_eq!(missing_reason.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&missing_reason.stderr).contains("warning[suppression-reason]")
    );
}

#[test]
fn explanations_are_available_without_a_project() {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rake"))
        .args(["lint", "--explain", "view-loop-index-id", "--json"])
        .current_dir(directory.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(json["rule"]["id"], "view-loop-index-id");
    assert_eq!(json["rule"]["default_level"], "warn");
    assert!(json["rule"]["before"].as_str().unwrap().contains("index"));
    assert!(json["rule"]["after"].as_str().unwrap().contains("row.id"));
    let output = run("", "", &["--explain", "view-image-alt"]);

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Before:\n<img"));
}

#[test]
fn invalid_arguments_and_config_use_existing_json_error_shape() {
    for arguments in [
        vec!["--explain", "unknown", "--json"],
        vec!["--explain"],
        vec!["--unknown", "--json"],
        vec!["first", "second", "--json"],
    ] {
        let output = run("", "", &arguments);

        assert_eq!(output.status.code(), Some(1), "{arguments:?}");
        assert!(output.stdout.is_empty());

        if arguments.contains(&"--json") {
            let json: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();

            assert!(json["error"]["message"].is_string());
        }
    }

    let output = run(
        "",
        "[package.metadata.rake.lint.levels]\nview-typo='warn'",
        &["--json"],
    );

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown lint rule view-typo"));
}

#[test]
fn malformed_source_is_an_error_and_help_is_successful() {
    let output = run("fn {", "", &["--json"]);

    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(report["diagnostics"][0]["rule_id"], "syntax-error");
    assert!(run("", "", &["--help"]).status.success());
}
