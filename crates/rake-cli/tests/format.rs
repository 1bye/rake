//! Covers: project, file, stdin, alias, check diffs, invalid markup and argument errors.
//! Real: rake binary, cargo fmt, rustfmt, filesystem and diff.
//! Doubles: temporary dependency-free Cargo projects.
//! Requires: Cargo, pinned rustfmt and diff on PATH; no display or GPU.

use std::fs;
use std::io::Write;
use std::process::{Command, Output, Stdio};

fn project() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir(directory.path().join("src")).unwrap();
    fs::write(
        directory.path().join("Cargo.toml"),
        "[package]\nname='format-fixture'\nversion='0.1.0'\nedition='2024'\n",
    )
    .unwrap();
    fs::write(
        directory.path().join("rustfmt.toml"),
        "max_width=80\ntab_spaces=2\nhard_tabs=false\n",
    )
    .unwrap();
    fs::write(
        directory.path().join("src/main.rs"),
        "fn main(){let _=view!{<div><span>\"Saved\"</span></div>};}\n",
    )
    .unwrap();
    directory
}

fn run(directory: &std::path::Path, args: &[&str], input: Option<&str>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rake"))
        .args(args)
        .current_dir(directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = input {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    } else {
        drop(child.stdin.take());
    }
    child.wait_with_output().unwrap()
}

#[test]
fn check_reports_diff_without_writing_then_format_reaches_a_fixed_point() {
    let directory = project();
    let path = directory.path().join("src/main.rs");
    let before = fs::read_to_string(&path).unwrap();
    let check = run(directory.path(), &["format", "--check"], None);
    assert_eq!(check.status.code(), Some(1), "{check:?}");
    assert!(
        String::from_utf8_lossy(&check.stdout).contains("<div>"),
        "{check:?}"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), before);

    let format = run(directory.path(), &["fmt"], None);
    assert!(format.status.success(), "{format:?}");
    let after = fs::read_to_string(&path).unwrap();
    assert!(after.contains("fn main() {"));
    assert!(
        after.contains("\n    <div>\n      <span>\"Saved\"</span>\n    </div>"),
        "{after}"
    );
    let check = run(directory.path(), &["format", "--check"], None);
    assert!(check.status.success(), "{check:?}");
    assert!(check.stdout.is_empty(), "{check:?}");
    assert_eq!(fs::read_to_string(path).unwrap(), after);
}

#[test]
fn stdin_is_a_complete_rust_analyzer_formatter() {
    let directory = project();
    let source =
        "async fn f(){let x=view!{<div><span>\"hello\"</span></div>}.into_any_element();}\n";
    let first = run(
        directory.path(),
        &["format", "--stdin", "--edition", "2024", "--emit", "stdout"],
        Some(source),
    );
    assert!(first.status.success(), "{first:?}");
    assert!(first.stderr.is_empty(), "{first:?}");
    let output = String::from_utf8(first.stdout).unwrap();
    assert!(output.starts_with("async fn f() {"));
    assert!(
        output.contains("\n    <div>\n      <span>\"hello\"</span>"),
        "{output}"
    );
    let again = run(
        directory.path(),
        &["fmt", "--stdin", "--edition", "2024"],
        Some(&output),
    );
    assert!(again.status.success(), "{again:?}");
    assert_eq!(again.stdout, output.as_bytes());
}

#[test]
fn explicit_files_do_not_write_siblings_and_check_has_a_unified_diff() {
    let directory = project();
    let sibling = directory.path().join("src/main.rs");
    let before = fs::read_to_string(&sibling).unwrap();
    fs::write(
        directory.path().join("selected.rs"),
        "fn selected(){view!{<span>\"x\"</span>};}\n",
    )
    .unwrap();
    let check = run(
        directory.path(),
        &["format", "selected.rs", "--check"],
        None,
    );
    assert_eq!(check.status.code(), Some(1));
    let diff = String::from_utf8(check.stdout).unwrap().replace('\\', "/");
    assert!(diff.starts_with("--- "), "{diff}");
    assert!(diff.contains("\n+++ "), "{diff}");
    assert!(diff.contains("\n@@ "), "{diff}");
    let format = run(directory.path(), &["format", "selected.rs"], None);
    assert!(format.status.success(), "{format:?}");
    assert_eq!(fs::read_to_string(sibling).unwrap(), before);
    assert!(
        run(directory.path(), &["fmt", "selected.rs", "--check"], None)
            .status
            .success()
    );
}

#[test]
fn invalid_view_is_reported_and_other_views_still_format_on_stdin() {
    let directory = project();
    let source = "fn main() { view! { <div><span></div> }; view!{<div><span/></div>}; }\n";
    let result = run(directory.path(), &["format", "--stdin"], Some(source));
    assert_eq!(result.status.code(), Some(1));
    let output = String::from_utf8(result.stdout).unwrap();
    let diagnostic = String::from_utf8(result.stderr).unwrap();
    assert!(output.contains("view! { <div><span></div> }"), "{output}");
    assert!(output.contains("<div>\n      <span/>"), "{output}");
    assert!(diagnostic.contains("<stdin>:2:"), "{diagnostic}");
    assert!(diagnostic.contains("expected closing"), "{diagnostic}");
}

#[test]
fn discovery_uses_lint_excludes_and_skips_trybuild_siblings() {
    let directory = project();
    let manifest = directory.path().join("Cargo.toml");
    let mut content = fs::read_to_string(&manifest).unwrap();
    content.push_str("[package.metadata.rake.lint]\nexclude=['fixtures']\n");
    fs::write(manifest, content).unwrap();
    fs::create_dir(directory.path().join("fixtures")).unwrap();
    fs::create_dir(directory.path().join("tests")).unwrap();
    fs::write(
        directory.path().join("fixtures/excluded.rs"),
        "fn bad(){view!{<div>};}",
    )
    .unwrap();
    fs::write(
        directory.path().join("tests/bad.rs"),
        "fn bad(){view!{<div>};}",
    )
    .unwrap();
    fs::write(directory.path().join("tests/bad.stderr"), "intentional").unwrap();
    let result = run(directory.path(), &["format"], None);
    assert!(result.status.success(), "{result:?}");
    assert!(
        fs::read_to_string(directory.path().join("fixtures/excluded.rs"))
            .unwrap()
            .contains("<div>")
    );
}

#[test]
fn invalid_arguments_do_not_write_anything_and_help_succeeds() {
    let directory = project();
    let original = fs::read_to_string(directory.path().join("src/main.rs")).unwrap();
    for args in [
        vec!["format", "--unknown"],
        vec!["format", "--stdin", "--check"],
        vec!["format", "--stdin", "src/main.rs"],
        vec!["format", "--edition"],
        vec!["format", "--emit", "files"],
        vec!["format", "Cargo.toml"],
        vec!["format", "src/main.rs", "missing.rs"],
    ] {
        let result = run(directory.path(), &args, None);
        assert_eq!(result.status.code(), Some(1), "{args:?}: {result:?}");
        assert!(result.stdout.is_empty(), "{args:?}: {result:?}");
    }
    assert_eq!(
        fs::read_to_string(directory.path().join("src/main.rs")).unwrap(),
        original
    );
    assert!(
        run(directory.path(), &["format", "--help"], None)
            .status
            .success()
    );
}

#[test]
fn project_formats_workspace_members_examples_and_nested_qualified_views() {
    let directory = project();
    let manifest = directory.path().join("Cargo.toml");
    let mut content = fs::read_to_string(&manifest).unwrap();
    content.push_str("[workspace]\nmembers=['crates/member']\nresolver='3'\n");
    fs::write(manifest, content).unwrap();
    fs::create_dir_all(directory.path().join("crates/member/src")).unwrap();
    fs::write(
        directory.path().join("crates/member/Cargo.toml"),
        "[package]\nname='format-member'\nversion='0.1.0'\nedition='2024'\n",
    )
    .unwrap();
    fs::create_dir_all(directory.path().join("examples/nested")).unwrap();
    let source =
        "fn render(){let _=rake::view!{<div>{Some(rake::view!{<span id=\"nested\"/>})}</div>};}\n";
    let paths = ["crates/member/src/lib.rs", "examples/nested/main.rs"];

    for path in paths {
        fs::write(directory.path().join(path), source).unwrap();
    }

    let check = run(directory.path(), &["format", "--check"], None);

    assert_eq!(check.status.code(), Some(1), "{check:?}");
    let diff = String::from_utf8(check.stdout).unwrap().replace('\\', "/");
    for path in paths {
        assert!(diff.contains(path), "{diff}");
        assert_eq!(
            fs::read_to_string(directory.path().join(path)).unwrap(),
            source
        );
    }
    let format = run(directory.path(), &["format"], None);
    assert!(format.status.success(), "{format:?}");
    let formatted: Vec<_> = paths
        .iter()
        .map(|path| fs::read_to_string(directory.path().join(path)).unwrap())
        .collect();

    for output in &formatted {
        assert_eq!(output.matches("rake::view! {").count(), 2, "{output}");
        assert!(output.contains("<div>\n"), "{output}");
        assert!(output.contains("<span id=\"nested\" />"), "{output}");
    }

    let check = run(directory.path(), &["fmt", "--check"], None);
    assert!(check.status.success(), "{check:?}");
    assert!(check.stdout.is_empty(), "{check:?}");

    for (path, output) in paths.iter().zip(formatted) {
        assert_eq!(
            fs::read_to_string(directory.path().join(path)).unwrap(),
            output
        );
    }
}
