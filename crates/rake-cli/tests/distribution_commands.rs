//! Real portable command consumers; no signing, network or platform packaging tools.
use std::{fs, process::Command};
#[test]
fn native_build_forwards_features_and_emits_clean_json() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(root.path().join("Cargo.toml"),"[workspace]\n[package]\nname='native-build-consumer'\nversion='1.0.0'\nedition='2024'\n[features]\nsmoke=[]\n").unwrap();
    fs::write(
        root.path().join("src/main.rs"),
        "fn main(){println!(\"feature={}\",cfg!(feature=\"smoke\"));}",
    )
    .unwrap();
    assert!(
        Command::new("cargo")
            .args(["generate-lockfile", "--offline", "--manifest-path"])
            .arg(root.path().join("Cargo.toml"))
            .status()
            .unwrap()
            .success()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_rake"))
        .args([
            "build",
            "--native",
            "--features",
            "smoke",
            "--manifest-path",
        ])
        .arg(root.path().join("Cargo.toml"))
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let binary = result["artifact"].as_str().unwrap();
    let run = Command::new(binary)
        .current_dir(std::env::temp_dir())
        .output()
        .unwrap();
    assert!(run.status.success());
    assert_eq!(
        String::from_utf8(run.stdout).unwrap().trim(),
        "feature=true"
    );
}
#[test]
fn distribution_doctor_reports_cross_host_requirements_without_building() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(root.path().join("Cargo.toml"),"[workspace]\n[package]\nname='doctor-consumer'\nversion='1.0.0'\nedition='2024'\n[package.metadata.rake.bundle]\nschema_version=1\n").unwrap();
    fs::write(root.path().join("src/main.rs"), "fn main(){}").unwrap();
    assert!(
        Command::new("cargo")
            .args(["generate-lockfile", "--offline", "--manifest-path"])
            .arg(root.path().join("Cargo.toml"))
            .status()
            .unwrap()
            .success()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_rake"))
        .args([
            "doctor",
            "--distribution",
            "--platform",
            "linux",
            "--format",
            "flatpak",
            "--manifest-path",
        ])
        .arg(root.path().join("Cargo.toml"))
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["capabilities"][0]["format"], "flatpak");
    assert_eq!(result["capabilities"][0]["native_host_required"], "linux");
    assert!(!root.path().join("dist").exists());
}

#[test]
fn command_recipe_and_custom_backend_use_the_versioned_stdio_contract() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("src")).unwrap();
    fs::create_dir(root.path().join("vendor")).unwrap();
    fs::write(
        root.path().join("vendor/original.txt"),
        "source stays unchanged",
    )
    .unwrap();
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let python = if cfg!(windows) { "python" } else { "python3" };
    let tool = root.path().join("extension.py");
    fs::write(
        &tool,
        r#"
import hashlib,json,pathlib,shutil,sys
request=json.load(sys.stdin)
assert request['schema_version']==1
mode=sys.argv[1]
if mode=='recipe':
    target=pathlib.Path(request['output'])/'runtime'
    shutil.copytree(request['input'],target)
    (target/'info.txt').write_text(request['options']['message'])
    print(json.dumps(dict(schema_version=1,root='runtime',native_code=[])))
elif mode=='package':
    assert pathlib.Path(request['input']).exists()
    target=pathlib.Path(request['output'])/'company installer.pkg'
    target.write_text(json.dumps(dict(identifier=request['identifier'],version=request['version'])))
    print(json.dumps(dict(schema_version=1,artifacts=[target.name])))
elif mode=='verify':
    actual=hashlib.file_digest(pathlib.Path(request['artifact']).open('rb'),'sha256').hexdigest()
    assert actual==request['sha256']
    print(json.dumps(dict(schema_version=1,sha256=actual,policy='fixture-format-contract')))
else:raise AssertionError(mode)
"#,
    )
    .unwrap();
    let (platform, format) = if cfg!(target_os = "macos") {
        ("macos", "app")
    } else if cfg!(windows) {
        ("windows", "zip")
    } else {
        ("linux", "tar")
    };
    let argv = |mode: &str| serde_json::to_string(&[python, tool.to_str().unwrap(), mode]).unwrap();
    let manifest = format!(
        "[workspace]\n[package]\nname='extension-consumer'\nversion='1.0.0'\nedition='2024'\n[dependencies]\nrake-resources={{path={:?}}}\n[build-dependencies]\nrake-build={{path={:?}}}\n[package.metadata.rake.bundle]\nschema_version=1\nidentifier='dev.rake.extension-consumer'\ncustom_formats=[{{name='company-installer',platform='{platform}',input_format='{format}',command={},verify_command={}}}]\nruntimes=[{{name='codec',recipe='codec',source='vendor',options={{message='prepared runtime'}}}}]\n[package.metadata.rake.bundle.component_recipes]\ncodec={}\n[package.metadata.rake.bundle.{platform}]\nformats=['{format}']\n",
        repository.join("rake-resources"),
        repository.join("rake-build"),
        argv("package"),
        argv("verify"),
        argv("recipe")
    );
    fs::write(root.path().join("Cargo.toml"), manifest).unwrap();
    fs::write(
        root.path().join("build.rs"),
        "fn main(){rake_build::build().unwrap();}",
    )
    .unwrap();
    fs::write(root.path().join("src/main.rs"),"fn main(){let resources=rake_resources::resources!().unwrap();println!(\"{}\",std::fs::read_to_string(resources.runtime(\"codec\").unwrap().join(\"info.txt\")).unwrap());}").unwrap();
    assert!(
        Command::new("cargo")
            .args(["generate-lockfile", "--offline", "--manifest-path"])
            .arg(root.path().join("Cargo.toml"))
            .status()
            .unwrap()
            .success()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_rake"))
        .args(["bundle", "--native", "--debug", "--manifest-path"])
        .arg(root.path().join("Cargo.toml"))
        .arg("--output")
        .arg(root.path().join("dist"))
        .arg("--json")
        .env(
            "CARGO_TARGET_DIR",
            repository.join("../target/distribution-tests"),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let custom = &result["custom_artifacts"][0];
    assert_eq!(custom["format"], "company-installer");
    assert_eq!(custom["verification"]["policy"], "fixture-format-contract");
    assert_eq!(custom["sha256"], custom["verification"]["sha256"]);
    assert!(
        root.path()
            .join("dist")
            .join(custom["path"].as_str().unwrap())
            .is_file()
    );
    assert!(!root.path().join("vendor/info.txt").exists());
}
