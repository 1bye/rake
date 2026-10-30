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

#[cfg(target_os = "macos")]
#[test]
fn common_metadata_and_localizations_reach_a_real_signed_application() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(root.path().join("src/main.rs"), "fn main(){}").unwrap();
    fs::write(
        root.path().join("Cargo.toml"),
        r#"[workspace]
[package]
name='metadata-consumer'
version='1.0.0'
edition='2024'
[package.metadata.rake.bundle]
schema_version=1
identifier='dev.rake.metadata-consumer'
[package.metadata.rake.bundle.desktop]
category='developer_tool'
url_schemes=['rake-metadata']
file_associations=[{extensions=['rnote'],name='Rake Note',mime_type='application/x-rake-note'}]
[package.metadata.rake.bundle.desktop.localizations.fr]
name='Notes françaises'
[package.metadata.rake.bundle.macos.privacy]
NSCameraUsageDescription='Capture a document image'
"#,
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
    let output = root.path().join("dist");
    let result = Command::new(env!("CARGO_BIN_EXE_rake"))
        .args([
            "bundle",
            "--native",
            "--debug",
            "--format",
            "app",
            "--json",
            "--manifest-path",
        ])
        .arg(root.path().join("Cargo.toml"))
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    let app = output.join(result["artifacts"][0]["path"].as_str().unwrap());
    let plist = Command::new("plutil")
        .args(["-convert", "json", "-o", "-"])
        .arg(app.join("Contents/Info.plist"))
        .output()
        .unwrap();
    assert!(
        plist.status.success(),
        "{}",
        String::from_utf8_lossy(&plist.stderr)
    );
    let plist: serde_json::Value = serde_json::from_slice(&plist.stdout).unwrap();
    assert_eq!(
        plist["LSApplicationCategoryType"],
        "public.app-category.developer-tools"
    );
    assert_eq!(
        plist["NSCameraUsageDescription"],
        "Capture a document image"
    );
    assert_eq!(
        plist["CFBundleURLTypes"][0]["CFBundleURLSchemes"][0],
        "rake-metadata"
    );
    assert_eq!(
        plist["CFBundleDocumentTypes"][0]["CFBundleTypeExtensions"][0],
        "rnote"
    );
    let bytes = fs::read(app.join("Contents/Resources/fr.lproj/InfoPlist.strings")).unwrap();
    let text = String::from_utf16(
        &bytes
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert!(text.contains("Notes françaises"));
    assert!(
        Command::new("codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(app)
            .status()
            .unwrap()
            .success()
    );
}

#[test]
fn repository_export_and_promotion_use_the_owned_release_worker() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(root.path().join("src/main.rs"), "fn main(){}").unwrap();
    fs::write(
        root.path().join("Cargo.toml"),
        r#"[workspace]
[package]
name="release-consumer"
version="1.0.0"
edition="2024"
[package.metadata.rake.bundle]
schema_version=1
identifier="dev.rake.release-consumer"
[package.metadata.rake.bundle.release.publisher]
kind="directory"
path="published"
public_url="https://downloads.example.test"
[package.metadata.rake.bundle.release.required]
x86_64-unknown-linux-gnu=["tar"]
"#,
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
    let input = root.path().join("input");
    fs::create_dir(&input).unwrap();
    fs::write(input.join("fixture.tar.gz"), []).unwrap();
    fs::write(input.join("distribution.json"),serde_json::to_vec(&serde_json::json!({
        "schema_version":1,"identifier":"dev.rake.release-consumer","version":"1.0.0","target":"x86_64-unknown-linux-gnu","engine":"protocol-fixture",
        "artifacts":[{"path":"fixture.tar.gz","format":"tar","sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","package_version":"1.0.0"}]
    })).unwrap()).unwrap();
    let export = root.path().join("repositories");
    for flag in ["--prepare-repositories", "--repositories"] {
        let output = Command::new(env!("CARGO_BIN_EXE_rake"))
            .args(["release", "--manifest-path"])
            .arg(root.path().join("Cargo.toml"))
            .arg("--from")
            .arg(&input)
            .arg(flag)
            .arg(&export)
            .arg("--json")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        if flag == "--prepare-repositories" {
            assert_eq!(result["publication"], "not published");
            assert!(export.join("repositories.json").is_file());
            assert!(!root.path().join("published").exists());
        } else {
            assert_eq!(result["version"], "1.0.0");
            assert!(root.path().join("published/stable/release.json").is_file());
        }
    }
}
