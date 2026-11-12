//! Run the bundled pinned Dylint library and decode its source diagnostics.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{Diagnostic, Position, Result, SourceSpan, catalogue};

const NIGHTLY: &str = "nightly-2026-08-20";
const INSTALL: &str = "rustup toolchain install nightly-2026-08-20 --profile minimal --component rustc-dev,llvm-tools-preview,rust-src,rustfmt\ncargo +nightly-2026-08-20 install --locked cargo-dylint --version 6.3.2\ncargo +nightly-2026-08-20 install --locked dylint-link --version 6.3.2";
const LIBRARY: &[(&str, &str)] = &[
    (
        "Cargo.toml",
        include_str!("../../lints/rake-gpui/Cargo.toml"),
    ),
    (
        "Cargo.lock",
        include_str!("../../lints/rake-gpui/Cargo.lock"),
    ),
    (
        "rust-toolchain.toml",
        include_str!("../../lints/rake-gpui/rust-toolchain.toml"),
    ),
    (
        ".cargo/config.toml",
        include_str!("../../lints/rake-gpui/.cargo/config.toml"),
    ),
    (
        "src/lib.rs",
        include_str!("../../lints/rake-gpui/src/lib.rs"),
    ),
];

pub(super) type Findings = BTreeMap<PathBuf, Vec<Diagnostic>>;

fn prerequisites() -> Result<()> {
    let nightly = Command::new("rustup")
        .args(["run", NIGHTLY, "rustc", "--version"])
        .output();
    let dylint = Command::new("cargo").args(["dylint", "--version"]).output();

    if !nightly.is_ok_and(|output| output.status.success())
        || !dylint.is_ok_and(|output| output.status.success())
    {
        return Err(format!(
            "deep lint requires the pinned nightly and cargo-dylint. Install explicitly:\n{INSTALL}"
        )
        .into());
    }

    Ok(())
}

// Bundle the source with the CLI so installed binaries need no Rake repository checkout.
// A content-addressed directory avoids rebuilding an unchanged library on each run.
fn library(root: &Path) -> Result<PathBuf> {
    let mut hash = Sha256::new();

    for (name, text) in LIBRARY {
        hash.update(name.as_bytes());
        hash.update(text.as_bytes());
    }

    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    let target = if target.is_absolute() {
        target
    } else {
        std::env::current_dir()?.join(target)
    };
    let directory = target
        .join("rake-lints")
        .join(format!("{:x}", hash.finalize()));

    for (name, text) in LIBRARY {
        let path = directory.join(name);

        if fs::read_to_string(&path).ok().as_deref() != Some(text) {
            fs::create_dir_all(path.parent().expect("bundled file has a parent"))?;
            fs::write(path, text)?;
        }
    }

    Ok(directory)
}

pub(super) fn collect(root: &Path) -> Result<Findings> {
    prerequisites()?;

    let metadata = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version=1", "--locked"])
        .current_dir(root)
        .output()?;

    if !metadata.status.success() {
        return Err(format!(
            "deep lint Cargo metadata failed:\n{}",
            String::from_utf8_lossy(&metadata.stderr)
        )
        .into());
    }

    let metadata: Value = serde_json::from_slice(&metadata.stdout)?;
    let workspace = Path::new(
        metadata["workspace_root"]
            .as_str()
            .ok_or("Cargo metadata omitted workspace_root")?,
    )
    .canonicalize()?;
    let library = library(&workspace)?;
    let mut command = Command::new("cargo");

    command
        .args(["dylint", "--path"])
        .arg(library)
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .args(["--", "--all-targets", "--locked", "--message-format=json"])
        .current_dir(root)
        .env("RUSTUP_AUTO_INSTALL", "0");

    if workspace == root {
        command.arg("--workspace");
    }

    // Keep rustc warning policy from bypassing reasoned comment suppressions or
    // turning a finding into a build failure before shared levels are applied.
    let mut flags = std::env::var("DYLINT_RUSTFLAGS").unwrap_or_default();

    for rule in catalogue::RULES
        .iter()
        .filter(|rule| catalogue::is_deep(rule.id))
    {
        flags.push_str(&format!(" --force-warn={}", rule.id.replace('-', "_")));
    }

    let output = command.env("DYLINT_RUSTFLAGS", flags).output()?;
    let (findings, built, errors) = decode(root, &workspace, &String::from_utf8(output.stdout)?)?;

    if !output.status.success() || !built {
        return Err(format!("deep lint did not complete Cargo verification:\n{}\n{errors}\nRequired tools (never installed automatically):\n{INSTALL}", String::from_utf8_lossy(&output.stderr)).into());
    }

    Ok(findings)
}

fn decode(root: &Path, workspace: &Path, output: &str) -> Result<(Findings, bool, String)> {
    let mut findings = Findings::new();
    let mut seen = BTreeSet::new();
    let mut built = false;
    let mut errors = String::new();

    for line in output.lines().filter(|line| line.starts_with('{')) {
        let event: Value = serde_json::from_str(line)?;

        if event["reason"] == "build-finished" {
            built = event["success"] == true;
        }

        if event["reason"] != "compiler-message" {
            continue;
        }

        let message = &event["message"];

        if message["level"] == "error" {
            errors.push_str(message["rendered"].as_str().unwrap_or("compiler error"));
        }

        let Some(code) = message["code"]["code"].as_str() else {
            continue;
        };
        let id = code.replace('_', "-");
        let Some(rule) = catalogue::rule(&id).filter(|rule| catalogue::is_deep(rule.id)) else {
            continue;
        };

        for span in message["spans"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|span| span["is_primary"] == true)
        {
            let Some(file) = span["file_name"].as_str() else {
                continue;
            };
            let path = Path::new(file);
            // Cargo emits workspace-relative paths, even for a selected member.
            let path = if path.is_absolute() {
                path.to_owned()
            } else {
                workspace.join(path)
            };
            let Ok(path) = path.canonicalize() else {
                continue;
            };
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            let position = |prefix: &str| -> Result<Position> {
                Ok(Position {
                    line: span[format!("line_{prefix}")]
                        .as_u64()
                        .ok_or("Dylint span omitted line")? as usize,
                    column: span[format!("column_{prefix}")]
                        .as_u64()
                        .ok_or("Dylint span omitted column")? as usize,
                })
            };
            let source_span = SourceSpan {
                start: position("start")?,
                end: position("end")?,
            };
            let key = (
                path.clone(),
                rule.id,
                source_span.start.line,
                source_span.start.column,
                source_span.end.line,
                source_span.end.column,
            );

            // A source can be compiled as both a library and a test target.
            if !seen.insert(key) {
                continue;
            }

            let mut diagnostic = Diagnostic::new(rule.id, relative, proc_macro2::Span::call_site());

            diagnostic.span = source_span;
            findings.entry(path).or_default().push(diagnostic);
        }
    }

    Ok((findings, built, errors))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn compiler_messages_keep_unicode_columns_and_deduplicate_targets() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();

        fs::write(root.join("main.rs"), "fn f() { let é = task(); }\n").unwrap();
        let event = json!({"reason":"compiler-message", "message": {
            "level":"warning", "code":{"code":"gpui_dropped_task"},
            "spans":[{"file_name":"main.rs", "is_primary":true,
            "line_start":1,"line_end":1,"column_start":18,"column_end":24}]
        }})
        .to_string();
        let events =
            format!("{event}\n{event}\n{{\"reason\":\"build-finished\",\"success\":true}}\n");
        let (findings, built, errors) = decode(&root, &root, &events).unwrap();
        let diagnostics = &findings[&root.join("main.rs")];

        assert!(built);
        assert!(errors.is_empty());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule_id, catalogue::DROPPED_TASK);
        assert_eq!(diagnostics[0].span.start.column, 18);
        assert_eq!(diagnostics[0].file, "main.rs");
    }

    #[test]
    fn failure_and_empty_output_cannot_look_like_a_successful_check() {
        let root = Path::new(".");
        let (_, built, _) = decode(root, root, "").unwrap();

        assert!(!built);
        let (_, built, errors) = decode(
            root,
            root,
            &serde_json::json!({
                "reason":"compiler-message", "message":{"level":"error","rendered":"type error"}
            })
            .to_string(),
        )
        .unwrap();

        assert!(!built);
        assert_eq!(errors, "type error");
        assert!(decode(root, root, "{broken json").is_err());
    }
}
