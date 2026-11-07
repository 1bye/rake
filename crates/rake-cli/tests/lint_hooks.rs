//! Covers: built-in PostToolUse parsing, compact feedback, limits and advisory exits.
//! Real: rake binary, source/config parsing and JSON; no external adapters.
//! Doubles: representative Claude Code and Codex event payloads.
//! Requires: no network, GUI, agent account or external executable.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn invoke(cwd: &Path, payload: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rake"))
        .args(["lint", "--hook"])
        .current_dir(cwd)
        .env("PATH", "")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(payload).unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success(), "{result:?}");
    assert!(result.stderr.is_empty(), "{result:?}");
    result
}

fn hook(process_cwd: &Path, cwd: Option<&Path>, tool: &str, input: serde_json::Value) -> Output {
    let mut event = serde_json::json!({
        "hook_event_name": "PostToolUse", "tool_name": tool, "tool_input": input,
    });
    if let Some(cwd) = cwd {
        event["cwd"] = serde_json::json!(cwd);
    }
    invoke(process_cwd, event.to_string().as_bytes())
}

fn context(result: &Output) -> String {
    let output: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(output.as_object().unwrap().len(), 1);
    assert_eq!(output["hookSpecificOutput"]["hookEventName"], "PostToolUse");
    output["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .into()
}

fn project() -> tempfile::TempDir {
    let project = tempfile::tempdir().unwrap();
    fs::write(
        project.path().join("Cargo.toml"),
        "[package]\nname='app'\nversion='0.1.0'",
    )
    .unwrap();
    project
}

#[test]
fn claude_edit_and_write_resolve_payload_cwd_and_return_compact_findings() {
    let project = project();
    let root = project.path();
    let process_cwd = tempfile::tempdir().unwrap();
    let file = root.join("space ü.rs");
    let original = "fn f() { rake::view! { <button/> }; }";
    fs::write(&file, original).unwrap();
    for tool in ["Edit", "Write"] {
        let result = hook(
            process_cwd.path(),
            Some(root),
            tool,
            serde_json::json!({"file_path": "space ü.rs"}),
        );
        let context = context(&result);
        assert!(context.starts_with("rake lint found 1 issue in the edited files:\n"));
        let line = context.lines().nth(1).unwrap();
        assert!(line.starts_with("space ü.rs:1:"), "{line}");
        assert!(line.contains("view-button-label (warn):"));
        assert!(line.contains(". Fix: add aria-label="));
        assert!(!context.contains("Rake automatically labels"));
        assert_eq!(context.lines().count(), 2);
        assert_eq!(fs::read_to_string(&file).unwrap(), original);
    }
}

#[test]
fn codex_patch_handles_spaces_multiple_files_move_delete_and_duplicates() {
    let project = project();
    let root = project.path();
    for name in ["added space.rs", "updated space.rs", "moved ü.rs"] {
        fs::write(root.join(name), "fn f() { rake::view! { <button/> }; }").unwrap();
    }
    fs::write(root.join("unrelated.rs"), "fn { broken source").unwrap();
    let patch = "*** Begin Patch\n*** Add File: added space.rs\n+...\n*** Update File: added space.rs\n@@\n-...\n+...\n*** Update File: updated space.rs\n@@\n-...\n+...\n*** Update File: old.rs\n*** Move to: moved ü.rs\n@@\n-...\n+...\n*** Delete File: deleted.rs\n*** End Patch\n";
    let result = hook(
        root,
        Some(root),
        "apply_patch",
        serde_json::json!({"command": patch}),
    );
    let context = context(&result);
    assert!(context.starts_with("rake lint found 3 issues"));
    assert_eq!(context.matches("view-button-label").count(), 3);
    assert_eq!(context.lines().count(), 4);
    assert!(!context.contains("unrelated.rs"));
    assert!(!context.contains("syntax-error"));
}

#[test]
fn missing_cwd_falls_back_to_the_process_directory() {
    let project = project();
    let root = project.path();
    fs::write(
        root.join("main.rs"),
        "fn f() { rake::view! { <button/> }; }",
    )
    .unwrap();
    let result = hook(
        root,
        None,
        "Edit",
        serde_json::json!({"file_path": "main.rs"}),
    );
    assert!(context(&result).contains("main.rs:1:"));
}

#[test]
fn clean_non_rust_and_deleted_files_have_empty_stdout() {
    let project = project();
    let root = project.path();
    fs::write(root.join("clean.rs"), "fn main() {}").unwrap();
    fs::write(root.join("notes.md"), "text").unwrap();
    for file in ["clean.rs", "notes.md", "deleted.rs"] {
        let result = hook(
            root,
            Some(root),
            "Write",
            serde_json::json!({"file_path": root.join(file)}),
        );
        assert!(result.stdout.is_empty(), "{result:?}");
    }
    let result = hook(
        root,
        Some(root),
        "apply_patch",
        serde_json::json!({"command": "*** Begin Patch\n*** Delete File: deleted.rs\n*** End Patch\n"}),
    );
    assert!(result.stdout.is_empty());
}

#[test]
fn malformed_unsupported_and_unreadable_payloads_return_advisory_context() {
    let project = project();
    let root = project.path();
    for input in [b"not json".as_slice(), b"{}", b"[]", b"\xff"] {
        let result = invoke(root, input);
        assert!(context(&result).contains("run rake lint manually"));
    }
    for (tool, input) in [
        ("Edit", serde_json::json!({})),
        ("Write", serde_json::json!({"file_path": 12})),
        ("Bash", serde_json::json!({"command": "anything"})),
        (
            "apply_patch",
            serde_json::json!({"command": "unknown patch format"}),
        ),
    ] {
        let result = hook(root, Some(root), tool, input);
        assert!(context(&result).contains("run rake lint manually"));
    }
}

#[test]
fn lint_failures_and_denied_findings_never_block_the_edit() {
    let project = project();
    let root = project.path();
    fs::write(root.join("main.rs"), "fn {").unwrap();
    let result = hook(
        root,
        Some(root),
        "Edit",
        serde_json::json!({"file_path": "main.rs"}),
    );
    assert!(context(&result).contains("syntax-error (deny):"));
    fs::write(root.join("Cargo.toml"), "not valid TOML").unwrap();
    let result = hook(
        root,
        Some(root),
        "Edit",
        serde_json::json!({"file_path": "main.rs"}),
    );
    let context = context(&result);
    assert!(context.contains("could not check an edited file; run rake lint manually"));
    assert!(!context.contains("schema_version"));
}

#[test]
fn findings_are_capped_at_twenty_with_the_remaining_count() {
    let project = project();
    let root = project.path();
    let source = format!(
        "fn f() {{ rake::view! {{ <div>{}</div> }}; }}",
        "<button/>\n".repeat(27)
    );
    fs::write(root.join("many.rs"), source).unwrap();
    let result = hook(
        root,
        Some(root),
        "Write",
        serde_json::json!({"file_path": "many.rs"}),
    );
    let context = context(&result);
    assert!(context.starts_with("rake lint found 27 issues"));
    assert_eq!(context.matches("view-button-label").count(), 20);
    assert!(context.ends_with("7 more; run rake lint"));
    assert_eq!(context.lines().count(), 22);
}

#[test]
fn manifest_findings_are_compact_and_read_only() {
    let project = project();
    let root = project.path();
    let manifest = root.join("Cargo.toml");
    fs::write(
        &manifest,
        "[package]\nname='app'\nversion='0.1.0'\n[profile.release]\ndebug-assertions=true # keep\n",
    )
    .unwrap();
    let result = hook(
        root,
        Some(root),
        "Write",
        serde_json::json!({"file_path": manifest}),
    );
    assert!(context(&result).contains("Cargo.toml:5:18 release-debug-assertions (warn):"));
    assert!(
        fs::read_to_string(root.join("Cargo.toml"))
            .unwrap()
            .contains("true # keep")
    );
}

#[test]
fn long_fields_are_shortened_without_losing_the_rule_and_fix() {
    let project = project();
    let root = project.path();
    let mut directory = root.to_owned();
    for ch in ['a', 'b', 'c'] {
        directory.push(ch.to_string().repeat(170));
        fs::create_dir(&directory).unwrap();
    }
    let path = directory.join("main.rs");
    fs::write(&path, "fn f() { rake::view! { <button/> }; }").unwrap();
    let result = hook(
        root,
        Some(root),
        "Edit",
        serde_json::json!({"file_path": path}),
    );
    let context = context(&result);
    let line = context.lines().nth(1).unwrap();
    assert!(line.contains("..."));
    assert!(line.contains("view-button-label (warn):"));
    assert!(line.contains(". Fix: add aria-label="));
    assert!(line.chars().count() < 600);
}
