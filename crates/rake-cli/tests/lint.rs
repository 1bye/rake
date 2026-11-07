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
    let output = run("", "", &["--explain", "view-button-label", "--json"]);
    let rule: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert!(output.status.success());
    assert!(
        rule["rule"]["explanation"]
            .as_str()
            .unwrap()
            .contains("visible text as the label (label-in-name); add context only after it")
    );
    assert!(
        rule["rule"]["after"]
            .as_str()
            .unwrap()
            .contains("aria-label={title.clone()}")
    );
    assert!(
        rule["rule"]["after"]
            .as_str()
            .unwrap()
            .contains("aria-label=\"Search\"")
    );
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

#[test]
fn compile_fail_fixtures_and_project_excludes_do_not_fail_the_command() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();

    fs::write(root.join("Cargo.toml"), "[package]\nname='lint-fixture'\nversion='0.1.0'\n[package.metadata.rake.lint]\nexclude=['tests/custom']\n").unwrap();
    fs::create_dir_all(root.join("tests/ui")).unwrap();
    fs::create_dir_all(root.join("tests/custom")).unwrap();
    fs::write(root.join("tests/ui/bad.rs"), "fn {").unwrap();
    fs::write(root.join("tests/ui/bad.stderr"), "intentional syntax error").unwrap();
    fs::write(root.join("tests/custom/bad.rs"), "fn {").unwrap();
    fs::write(root.join("main.rs"), "fn main() {}").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rake"))
        .args(["lint", "--json"])
        .current_dir(root)
        .env("PATH", "")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(report["files_checked"], 1);
    assert_eq!(report["diagnostics"], serde_json::json!([]));
    assert_eq!(report["denied"], false);
}

#[test]
fn agent_guide_recommends_formatting_without_requiring_a_project() {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rake"))
        .args(["lint", "--guide"])
        .current_dir(directory.path())
        .env("PATH", "")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("run rake format, then rake lint"));
}

#[test]
fn fix_rechecks_formats_preserves_comments_and_is_idempotent() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    fs::write(root.join("Cargo.toml"), "[package]\nname='lint-fix'\nversion='0.1.0'\nedition='2024'\n[profile.release]\ndebug-assertions=true # keep profile reason\n").unwrap();
    let source = "fn main() {\n    // keep before\n    rake::view! { <div id=\"stable\" style={|s| s.flex().p_4()} aria-label=\"named\"/> }; // keep after\n}\n";
    fs::write(root.join("main.rs"), source).unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_rake"))
            .args(args)
            .current_dir(root)
            .output()
            .unwrap()
    };
    let before = run(&["lint", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&before.stdout).unwrap();
    assert!(
        before.status.success(),
        "{}",
        String::from_utf8_lossy(&before.stderr)
    );
    assert_eq!(json["diagnostics"].as_array().unwrap().len(), 2);
    for diagnostic in json["diagnostics"].as_array().unwrap() {
        assert_eq!(
            diagnostic["machine_fix"]["applicability"],
            "machine-applicable"
        );
        assert!(diagnostic["machine_fix"]["span"]["start"]["column"].is_number());
    }

    let output = run(&["lint", "--fix", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["diagnostics"], serde_json::json!([]));
    let fixed = fs::read_to_string(root.join("main.rs")).unwrap();
    assert!(fixed.contains("id=\"stable\" class=\"flex p-4\" aria-label=\"named\""));
    assert!(fixed.contains("// keep before"));
    assert!(fixed.contains("// keep after"));
    syn::parse_file(&fixed).unwrap();
    let manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(manifest.contains("debug-assertions=false # keep profile reason"));
    let formatted = run(&["format", "main.rs", "--check"]);
    assert!(
        formatted.status.success(),
        "{}{}",
        String::from_utf8_lossy(&formatted.stdout),
        String::from_utf8_lossy(&formatted.stderr)
    );

    assert!(run(&["lint", "--fix", "--json"]).status.success());
    assert_eq!(fs::read_to_string(root.join("main.rs")).unwrap(), fixed);
    assert_eq!(
        fs::read_to_string(root.join("Cargo.toml")).unwrap(),
        manifest
    );
}

#[test]
fn fix_retains_unsafe_commented_suppressed_and_denied_findings() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='lint-fix'\nversion='0.1.0'\nedition='2024'",
    )
    .unwrap();
    let source = "fn main() { rake::view! {\n<div>\n<div style={|s| s /* keep inside */ .flex()}/>\n// rake-lint: allow(view-style-class) Demonstrate builders.\n<div style={|s| s.p_2()}/>\n<img src=\"a\"/>\n</div> }; }\n";
    fs::write(root.join("main.rs"), source).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rake"))
        .args(["lint", "--fix", "--json"])
        .current_dir(root)
        .env("PATH", "")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let diagnostics = json["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 2);
    assert!(diagnostics.iter().all(|d| d.get("machine_fix").is_none()));
    assert_eq!(fs::read_to_string(root.join("main.rs")).unwrap(), source);
}

#[test]
fn fix_works_with_unicode_crlf_and_attribute_order() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='lint-fix'\nversion='0.1.0'\nedition='2024'",
    )
    .unwrap();
    fs::write(root.join("main.rs"), "fn main() {\r\n    let _café = 1;\r\n    rake::view! { <div aria-label=\"你好\" style={|s| s.p_2()} id=\"last\"/> };\r\n}\r\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rake"))
        .args(["lint", "--fix", "--json"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let source = fs::read_to_string(root.join("main.rs")).unwrap();
    assert!(source.contains("aria-label=\"你好\" class=\"p-2\" id=\"last\""));
    let output = Command::new(env!("CARGO_BIN_EXE_rake"))
        .args(["format", "main.rs", "--check"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn fix_rejects_modes_that_do_not_check_a_project() {
    for args in [
        vec!["--guide", "--fix", "--json"],
        vec!["--explain", "view-style-class", "--fix", "--json"],
    ] {
        let output = run("", "", &args);
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert!(serde_json::from_slice::<serde_json::Value>(&output.stderr).unwrap()["error"]["message"].is_string());
    }
}

#[test]
fn file_suppressions_control_json_exit_status_and_machine_fixes() {
    let source = "// rake-lint: allow-file(view-button-label) Mounted widgets name their actions.\nfn main() { rake::view! { <div><button/><button/></div> }; }";
    let output = run(
        source,
        "[package.metadata.rake.lint.levels]\nview-button-label='deny'",
        &["--json"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["diagnostics"], serde_json::json!([]));

    let unused = run(
        "// rake-lint: allow-file(view-button-label) No longer needed.\nfn main() {}",
        "[package.metadata.rake.lint.levels]\ninvalid-suppression='deny'",
        &["--json"],
    );
    assert_eq!(unused.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&unused.stdout).unwrap();
    assert_eq!(report["diagnostics"][0]["rule_id"], "invalid-suppression");
    assert!(
        report["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .contains("unused file suppression")
    );
    assert_eq!(report["diagnostics"][0]["span"]["start"]["line"], 1);

    let project = tempfile::tempdir().unwrap();
    fs::write(
        project.path().join("Cargo.toml"),
        "[package]\nname='lint-fix'\nversion='0.1.0'",
    )
    .unwrap();
    let source = "// rake-lint: allow-file(view-style-class) Demonstrates fluent builders.\nfn main(){rake::view! { <div style={|s| s.flex()}/> };}\n";
    fs::write(project.path().join("main.rs"), source).unwrap();
    for _ in 0..2 {
        let output = Command::new(env!("CARGO_BIN_EXE_rake"))
            .args(["lint", "--fix", "--json"])
            .current_dir(project.path())
            .env("PATH", "")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["diagnostics"], serde_json::json!([]));
        assert_eq!(
            fs::read_to_string(project.path().join("main.rs")).unwrap(),
            source
        );
    }
}
