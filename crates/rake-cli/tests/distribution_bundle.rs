//! Covers: portable CLI packages a real tiny Cargo binary and preserves output ownership.
//! Real: Cargo, the selected native packaging engine, worker process and filesystem.
//! Doubles: tiny console app. Requires: native packaging tools for the selected host.
use std::{fs, process::Command};

#[test]
fn native_bundle_preserves_resources_and_refuses_unowned_output() {
    let root = std::env::temp_dir().join(format!("rake-native-cli-{}", std::process::id()));

    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }

    fs::create_dir_all(root.join("src")).unwrap();
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();

    let target = Command::new("rustc").arg("-vV").output().unwrap();
    let target = String::from_utf8(target.stdout).unwrap();

    let target = target
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .unwrap();

    fs::create_dir(root.join("vendor")).unwrap();
    fs::write(root.join("helper.rs"), "fn main(){print!(\"helper\");}").unwrap();
    let helper_name = if cfg!(windows) {
        "helper.exe"
    } else {
        "helper"
    };

    assert!(
        Command::new("rustc")
            .arg(root.join("helper.rs"))
            .arg("-o")
            .arg(root.join("vendor").join(helper_name))
            .status()
            .unwrap()
            .success()
    );
    let manifest = format!(
        "[workspace]\n[package]\nname='tiny-dist-fixture'\nversion='1.0.0'\nedition='2024'\n[package.metadata.rake.bundle]\nschema_version=1\nidentifier='dev.rake.fixture'\nresources=[{{from='message.txt',to='message.txt'}}]\ncomponents=[{{name='tools',target='{target}',source={{type='local',path='vendor'}}}}]\nsidecars=[{{name='test-helper',path='{helper_name}',component='tools'}}]\n[dependencies]\nrake-resources={{path={:?}}}\n[build-dependencies]\nrake-build={{path={:?}}}\n",
        repository.join("rake-resources"),
        repository.join("rake-build")
    );

    let manifest = if cfg!(windows) {
        fs::write(
            root.join("custom.manifest"),
            include_str!("../../rake-dist-windows/templates/default.manifest.xml"),
        )
        .unwrap();

        format!("{manifest}\n[package.metadata.rake.bundle.windows]\nmanifest='custom.manifest'\n")
    } else {
        manifest
    };

    fs::write(root.join("Cargo.toml"), manifest).unwrap();
    fs::write(
        root.join("build.rs"),
        "fn main(){rake_build::build().unwrap();}",
    )
    .unwrap();
    fs::write(root.join("src/main.rs"), r#"fn main(){let resources=rake_resources::resources!().unwrap();let helper=std::process::Command::new(resources.helper("test-helper").unwrap()).output().unwrap();print!("{}:{}",std::fs::read_to_string(resources.path("message.txt").unwrap()).unwrap(),String::from_utf8(helper.stdout).unwrap());}"#).unwrap();
    fs::write(root.join("message.txt"), "packaged resource").unwrap();
    assert!(
        Command::new("cargo")
            .args(["generate-lockfile", "--offline", "--manifest-path"])
            .arg(root.join("Cargo.toml"))
            .status()
            .unwrap()
            .success()
    );
    let format = if cfg!(target_os = "macos") {
        "app"
    } else if cfg!(windows) {
        "nsis"
    } else {
        "deb"
    };

    let output = root.join("distribution");

    let run = || {
        Command::new(env!("CARGO_BIN_EXE_rake"))
            .args([
                "bundle",
                "--native",
                "--debug",
                "--json",
                "--format",
                format,
                "--manifest-path",
            ])
            .arg(root.join("Cargo.toml"))
            .arg("--output")
            .arg(&output)
            .env(
                "CARGO_TARGET_DIR",
                repository.join("../target/distribution-tests"),
            )
            .output()
            .unwrap()
    };

    let result = run();

    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();

    assert_eq!(value["identifier"], "dev.rake.fixture");
    let artifact = output.join(value["artifacts"][0]["path"].as_str().unwrap());

    assert!(artifact.exists());

    if cfg!(target_os = "macos") {
        assert_eq!(
            fs::read_to_string(artifact.join("Contents/Resources/message.txt")).unwrap(),
            "packaged resource"
        );
        let binary = artifact.join("Contents/MacOS/tiny-dist-fixture");

        let launched = Command::new(binary)
            .current_dir(std::env::temp_dir())
            .output()
            .unwrap();

        assert_eq!(
            String::from_utf8(launched.stdout).unwrap().trim(),
            "packaged resource:helper"
        );
    }

    let before = fs::read(output.join("distribution.json")).unwrap();

    fs::copy(
        output.join("components.lock.json"),
        root.join("rake-dist.lock"),
    )
    .unwrap();
    fs::write(root.join("vendor/changed.txt"), "changed component input").unwrap();
    let changed = run();

    assert!(!changed.status.success());
    assert!(String::from_utf8_lossy(&changed.stderr).contains("rake-dist.lock"));
    assert_eq!(fs::read(output.join("distribution.json")).unwrap(), before);

    fs::remove_file(output.join(".rake-distribution")).unwrap();
    fs::write(output.join("keep.txt"), "not yours").unwrap();
    let refused = run();

    assert!(!refused.status.success());
    assert_eq!(
        fs::read_to_string(output.join("keep.txt")).unwrap(),
        "not yours"
    );
    fs::remove_dir_all(root).unwrap();
}
