//! Real doctor and console build through the project adapter on Windows and Unix.
//! Requires installed Rust/Cargo; no Python, Node, GPU, sockets or fixed temp paths.

use serde_json::Value;
use std::{fs, process::Command};

#[test]
fn doctor_has_no_python_dependency_or_diagnostic_field() {
    let output = Command::new(env!("CARGO_BIN_EXE_rake"))
        .args(["doctor", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let checks: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(checks["rust"].is_string());
    assert!(checks["cargo"].is_string());
    assert!(checks.get("python").is_none());
}

#[test]
fn console_build_works_with_spaces_and_without_unix_transport() {
    let root = tempfile::Builder::new()
        .prefix("rake portable ")
        .tempdir()
        .unwrap();
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(
        root.path().join("Cargo.toml"),
        "[package]\nname='portable-fixture'\nversion='0.1.0'\nedition='2024'\n[workspace]\n[package.metadata.rake.bundle]\nidentifier='dev.rake.portable'\nassets='assets'\n",
    )
    .unwrap();
    fs::create_dir(root.path().join("assets")).unwrap();
    fs::write(root.path().join("assets/message.txt"), "installed asset").unwrap();
    fs::write(
        root.path().join("src/main.rs"),
        "fn main(){println!(\"portable works\");}",
    )
    .unwrap();

    let build = || {
        Command::new(env!("CARGO_BIN_EXE_rake"))
            .current_dir(root.path())
            .env_remove("CARGO_TARGET_DIR")
            .env("CARGO_PROFILE_DEV_DEBUG", "0")
            .args(["build", "--debug", "--json"])
            .output()
            .unwrap()
    };

    let output = build();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    let run = Command::new(result["artifact"].as_str().unwrap())
        .output()
        .unwrap();

    assert!(run.status.success());
    assert_eq!(
        String::from_utf8(run.stdout).unwrap().replace("\r\n", "\n"),
        "portable works\n"
    );
    assert!(root.path().join("Cargo.lock").is_file());

    let installed = root.path().join("dist/portable-fixture/desktop");
    let marker = installed.join(".rake-build.json");

    assert!(marker.is_file());

    let assets = if cfg!(target_os = "macos") {
        "application/portable-fixture.app/Contents/Resources/assets"
    } else if cfg!(windows) {
        "application/assets"
    } else {
        "application/share/dev.rake.portable/assets"
    };

    assert_eq!(
        fs::read_to_string(installed.join(assets).join("message.txt")).unwrap(),
        "installed asset"
    );

    #[cfg(windows)]
    {
        let manifest: Value =
            serde_json::from_slice(&fs::read(installed.join("bundle.json")).unwrap()).unwrap();

        assert_eq!(manifest["platform"], "windows");
        assert_eq!(
            installed.join(manifest["artifact"].as_str().unwrap()),
            std::path::Path::new(result["artifact"].as_str().unwrap())
        );
        assert!(
            installed
                .join(manifest["archive"]["path"].as_str().unwrap())
                .is_file()
        );
    }

    let rebuilt = build();

    assert!(
        rebuilt.status.success(),
        "{}",
        String::from_utf8_lossy(&rebuilt.stderr)
    );

    fs::remove_file(marker).unwrap();

    let refused = build();

    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("not owned"));
    assert_eq!(
        fs::read_to_string(installed.join(assets).join("message.txt")).unwrap(),
        "installed asset"
    );
}

#[test]
fn web_bundle_uses_portable_pipes_loader_discovery_urls_and_zip_names() {
    use serde_json::json;
    let root = tempfile::Builder::new()
        .prefix("rake web portable ")
        .tempdir()
        .unwrap();
    let path = root.path();
    fs::create_dir(path.join("tools")).unwrap();
    fs::create_dir(path.join("web")).unwrap();
    fs::write(path.join("Cargo.toml"), "fixture").unwrap();
    fs::write(
        path.join("Cargo.lock"),
        "[[package]]\nname='wasm-bindgen'\nversion='0.2.100'\n",
    )
    .unwrap();
    fs::write(path.join("input.wasm"), b"\0asmfixture").unwrap();
    fs::write(
        path.join("web/index.html"),
        "<html><head></head><script type='module' src='/pkg/fixture.js'></script></html>",
    )
    .unwrap();
    let package = json!({"id":"portable-web-fixture","name":"fixture","version":"1.2.3","source":null,
        "manifest_path":path.join("Cargo.toml"),"features":{},"dependencies":[],
        "targets":[{"name":"fixture","kind":["cdylib"],"crate_types":["cdylib"]}],
        "metadata":{"rake":{"bundle":{"identifier":"dev.rake.portable","web":{"offline":true,"base_path":"/tools/"}}}}});
    fs::write(path.join("metadata.json"), json!({"workspace_root":path,"workspace_members":["portable-web-fixture"],"packages":[package]}).to_string()).unwrap();
    fs::write(path.join("artifact.json"), json!({"reason":"compiler-artifact","package_id":"portable-web-fixture","target":{"name":"fixture"},"filenames":[path.join("input.wasm")]}).to_string()).unwrap();
    fs::write(
        path.join("tool.rs"),
        r#"
use std::{fs,path::PathBuf};
fn main() {
    let root=PathBuf::from(std::env::var_os("RAKE_PORTABLE_FIXTURE").unwrap());
    let args:Vec<_>=std::env::args().collect();
    let name=PathBuf::from(&args[0]).file_stem().unwrap().to_string_lossy().into_owned();
    if name=="node" {
        let script=fs::read_to_string(&args[1]).unwrap();
        assert!(script.contains("await cliMain(process.argv.slice(2));"));
        fs::write(root.join("agent-script-path"),&args[1]).unwrap();
        println!("agent fixture");
    } else if name=="cargo" {
        let file=if args[1]=="metadata" {"metadata.json"} else {"artifact.json"};
        println!("{}",fs::read_to_string(root.join(file)).unwrap());
    } else if args[1]=="--version" { println!("wasm-bindgen 0.2.100"); }
    else {
        let out=PathBuf::from(&args[args.iter().position(|a|a=="--out-dir").unwrap()+1]);
        fs::create_dir_all(&out).unwrap();
        fs::copy(root.join("input.wasm"),out.join("fixture_bg.wasm")).unwrap();
        fs::write(out.join("fixture.js"),"export default async function init() {};").unwrap();
    }
}
"#,
    )
    .unwrap();
    let suffix = std::env::consts::EXE_SUFFIX;
    let cargo = path.join("tools").join(format!("cargo{suffix}"));
    assert!(
        Command::new("rustc")
            .arg("--edition=2024")
            .arg(path.join("tool.rs"))
            .arg("-o")
            .arg(&cargo)
            .status()
            .unwrap()
            .success()
    );
    fs::copy(
        &cargo,
        path.join("tools").join(format!("wasm-bindgen{suffix}")),
    )
    .unwrap();
    let mut paths = vec![path.join("tools")];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let output = Command::new(env!("CARGO_BIN_EXE_rake"))
        .current_dir(path)
        .env("PATH", std::env::join_paths(&paths).unwrap())
        .env("RAKE_PORTABLE_FIXTURE", path)
        .args(["bundle", "--target", "web", "--debug", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    let manifest: Value =
        serde_json::from_slice(&fs::read(result["manifest"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!(manifest["deployment"]["base_path"], "/tools/");
    assert_eq!(manifest["deployment"]["offline"], true);
    let site = std::path::Path::new(result["artifact"].as_str().unwrap());
    let worker = fs::read_to_string(site.join("rake-service-worker.js")).unwrap();
    assert!(worker.contains("/pkg/fixture_bg.wasm"));
    assert!(!worker.contains("%5C"));
    let mut archive =
        zip::ZipArchive::new(fs::File::open(result["archive"].as_str().unwrap()).unwrap()).unwrap();
    let mut loader = false;
    for index in 0..archive.len() {
        let entry = archive.by_index(index).unwrap();
        assert!(!entry.name().unwrap().contains('\\'));
        loader |= entry.name().unwrap().ends_with("/pkg/fixture.js");
    }
    assert!(loader);
    assert!(
        manifest["files"]
            .as_array()
            .unwrap()
            .iter()
            .all(|file| !file["path"].as_str().unwrap().contains('\\'))
    );
    #[cfg(windows)]
    {
        fs::copy(&cargo, path.join("tools/node.exe")).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_rake"))
            .current_dir(path)
            .env("PATH", std::env::join_paths(&paths).unwrap())
            .env("RAKE_PORTABLE_FIXTURE", path)
            .args(["agent", "--help"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            "agent fixture"
        );
        let script = fs::read_to_string(path.join("agent-script-path")).unwrap();
        assert!(
            !std::path::Path::new(&script).exists(),
            "client resource must be removed after exit"
        );
    }
}
