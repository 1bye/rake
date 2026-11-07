//! Covers: documented post-edit hook inputs and model-visible lint feedback.
//! Real: Python adapter, rake binary, source/config parsing and JSON.
//! Doubles: representative Claude Code and Codex event payloads.
//! Requires: Python 3; no agent account, network, GUI or Cargo invocation.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

fn hook(root: &Path, tool: &str, input: serde_json::Value) -> std::process::Output {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/rake-lint-hook.py");
    let mut child = Command::new("python3")
        .arg(script)
        .env("RAKE_LINT_BIN", env!("CARGO_BIN_EXE_rake"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let event = serde_json::json!({
        "hook_event_name": "PostToolUse", "cwd": root, "tool_name": tool, "tool_input": input,
    });
    child
        .stdin
        .take()
        .unwrap()
        .write_all(event.to_string().as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn claude_edit_and_write_return_findings_to_the_agent_and_keep_files_unchanged() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='app'\nversion='0.1.0'",
    )
    .unwrap();
    let file = root.join("space ü.rs");
    let original = "fn f() { rake::view! { <button/> }; }";
    fs::write(&file, original).unwrap();
    for tool in ["Edit", "Write"] {
        let result = hook(root, tool, serde_json::json!({"file_path": file}));
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let output: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        let context = output["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(context.contains("view-button-label"));
        assert_eq!(output["hookSpecificOutput"]["hookEventName"], "PostToolUse");
        assert_eq!(fs::read_to_string(&file).unwrap(), original);
    }
}

#[test]
fn codex_patch_handles_add_update_move_and_delete_without_scanning_unrelated_errors() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='app'\nversion='0.1.0'",
    )
    .unwrap();
    for name in ["added.rs", "moved.rs"] {
        fs::write(root.join(name), "fn f() { rake::view! { <button/> }; }").unwrap();
    }
    fs::write(root.join("unrelated.rs"), "fn { broken source").unwrap();
    let patch = "*** Begin Patch\n*** Add File: added.rs\n+...\n*** Update File: old.rs\n*** Move to: moved.rs\n@@\n-...\n+...\n*** Delete File: deleted.rs\n*** End Patch\n";
    let result = hook(root, "apply_patch", serde_json::json!({"command": patch}));
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    let context = output["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert_eq!(context.matches("view-button-label").count(), 2);
    assert!(!context.contains("unrelated.rs"));
    assert!(!context.contains("syntax-error"));
}

#[test]
fn hooks_are_quiet_for_clean_non_rust_and_deleted_files_but_surface_bad_payloads() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='app'\nversion='0.1.0'",
    )
    .unwrap();
    fs::write(root.join("clean.rs"), "fn main() {}").unwrap();
    fs::write(root.join("notes.md"), "text").unwrap();
    for file in ["clean.rs", "notes.md", "deleted.rs"] {
        let result = hook(
            root,
            "Write",
            serde_json::json!({"file_path": root.join(file)}),
        );
        assert!(result.status.success());
        assert!(
            result.stdout.is_empty(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
    }
    let result = hook(
        root,
        "apply_patch",
        serde_json::json!({"command": "unknown patch format"}),
    );
    let output: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert!(
        output["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("Run rake lint manually")
    );
}
