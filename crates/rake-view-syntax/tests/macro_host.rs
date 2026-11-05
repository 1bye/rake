//! Covers: real macro-host text spacing without proc-macro2 source-location features.
//! Real: Cargo, rustc, a tiny procedural macro, and the shared parser.
//! Doubles: temporary macro/consumer crates; no GPUI.
//! Requires: the current Rust toolchain and cached syn/quote dependencies.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn macro_host_preserves_spacing_without_source_location_feature() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    let syntax = env!("CARGO_MANIFEST_DIR");

    fs::create_dir_all(root.join("macros/src")).unwrap();
    fs::create_dir_all(root.join("app/src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers=['macros','app']\nresolver='3'\n",
    )
    .unwrap();
    fs::write(root.join("macros/Cargo.toml"), format!("[package]\nname='spacing-host'\nversion='0.0.0'\nedition='2024'\n[lib]\nproc-macro=true\n[dependencies]\nrake-view-syntax={{path={syntax:?}}}\nsyn={{version='2',features=['full']}}\nquote='1'\n")).unwrap();
    fs::write(
        root.join("macros/src/lib.rs"),
        r#"
extern crate proc_macro;
use rake_view_syntax::{Node, View};
#[proc_macro]
pub fn text(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let view = syn::parse::<View>(input).expect("valid view source");
    let text = view.root().children.iter().filter_map(|node| {
        if let Node::Text(text) = node { Some(text.value()) } else { None }
    }).collect::<Vec<_>>().join("|");
    quote::quote!(#text).into()
}
"#,
    )
    .unwrap();
    fs::write(root.join("app/Cargo.toml"), "[package]\nname='spacing-consumer'\nversion='0.0.0'\nedition='2024'\n[dependencies]\nspacing-host={path='../macros'}\n").unwrap();
    fs::write(
        root.join("app/src/main.rs"),
        r#"
fn main() {
    assert_eq!(spacing_host::text!(<p>Hello, world!</p>), "Hello, world!");
    assert_eq!(spacing_host::text!(<p>Hello {name} world!</p>), "Hello | world!");
    assert_eq!(spacing_host::text!(<p>你好 世界</p>), "你好 世界");
    println!("macro-host spacing preserved");
}
"#,
    )
    .unwrap();
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"))
        .join("view-syntax-macro-host");
    let cargo = Path::new(env!("CARGO"));
    let rustc = cargo.with_file_name(format!("rustc{}", std::env::consts::EXE_SUFFIX));
    let command = || {
        let mut command = Command::new(cargo);

        command
            .current_dir(root)
            .env("RUSTUP_AUTO_INSTALL", "0")
            .env("RUSTC", &rustc)
            .env("CARGO_TARGET_DIR", &target)
            .env("CARGO_BUILD_JOBS", "4");

        command
    };

    let output = command()
        .args(["run", "--offline", "--quiet", "-p", "spacing-consumer"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "macro-host spacing preserved"
    );
    let tree = command()
        .args([
            "tree",
            "--offline",
            "-e",
            "features",
            "-p",
            "spacing-consumer",
        ])
        .output()
        .unwrap();

    assert!(
        tree.status.success(),
        "{}",
        String::from_utf8_lossy(&tree.stderr)
    );
    assert!(
        !String::from_utf8_lossy(&tree.stdout).contains("span-locations"),
        "{}",
        String::from_utf8_lossy(&tree.stdout)
    );
}
