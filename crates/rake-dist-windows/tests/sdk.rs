//! Covers: real SDK manifest replacement, MSIX assembly, signing and managed upgrades.
//! Real: rustc, mt.exe, makeappx.exe, signtool and optional Windows AppX deployment.
//! Doubles: tiny console application. Requires: Windows SDK; signing suite also
//! requires RAKE_TEST_CERT_THUMBPRINT in a trusted disposable current-user store.
#![cfg(all(windows, feature = "native-tests"))]
#[cfg(feature = "native-signing-tests")]
use rake_dist_core::TrustOptions;
use rake_dist_windows::{Msix, apply_manifest, package_msix_with_metadata};
#[cfg(feature = "native-signing-tests")]
use rake_dist_windows::{certificate_subject, package_msix, sign};
use std::{fs, path::Path, process::Command};

fn run(command: &mut Command) -> String {
    let output = command.output().unwrap();

    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8_lossy(&output.stdout).trim().into()
}

fn compile(root: &Path, version: &str) {
    let source = root.join("main.rs");

    fs::write(&source, format!("fn main(){{print!(\"{version}\");}}")).unwrap();
    run(Command::new("rustc")
        .arg(&source)
        .arg("-o")
        .arg(root.join("fixture.exe")));
    let manifest = root.join("custom.manifest");

    fs::write(&manifest, include_str!("../templates/default.manifest.xml")).unwrap();
    apply_manifest(&root.join("fixture.exe"), &manifest).unwrap();
    fs::remove_file(source).unwrap();
    fs::remove_file(manifest).unwrap();
}

#[test]
fn sdk_validates_and_unpacks_a_real_managed_package() {
    let root = tempfile::tempdir().unwrap();
    let payload = root.path().join("payload");

    fs::create_dir(&payload).unwrap();
    compile(&payload, "1.0.0");
    let icons = rake_build::generate_icons(None, root.path()).unwrap();

    rake_build::generate_msix_assets(&icons, &payload).unwrap();
    let settings = Msix {
        identifier: "dev.rake.sdk.fixture",
        publisher: "CN=Rake SDK Test",
        version: "1.0.0.0",
        target: "x86_64-pc-windows-msvc",
        executable: "fixture.exe",
        name: "Rake SDK Fixture",
        manifest: None,
    };

    let metadata = rake_dist_core::DesktopMetadata {
        url_schemes: vec!["rake-sdk-test".into()],
        file_associations: vec![rake_dist_core::FileAssociation {
            extensions: vec!["rnote".into()],
            name: Some("Rake Note".into()),
            description: None,
            mime_type: Some("application/x-rake-note".into()),
        }],
        localizations: [(
            "fr".into(),
            rake_dist_core::LocalizedMetadata {
                name: Some("Notes françaises".into()),
                description: Some("Documents & notes".into()),
            },
        )]
        .into(),
        ..Default::default()
    };

    let package = package_msix_with_metadata(&payload, root.path(), &settings, &metadata).unwrap();
    let extracted = root.path().join("extracted");

    run(Command::new("makeappx.exe")
        .args(["unpack", "/o", "/p"])
        .arg(package)
        .arg("/d")
        .arg(&extracted));
    assert!(extracted.join("AppxManifest.xml").is_file());
    assert!(extracted.join("resources.pri").is_file());
    let manifest = fs::read_to_string(extracted.join("AppxManifest.xml")).unwrap();

    assert!(manifest.contains("windows.protocol"));
    assert!(manifest.contains("windows.fileTypeAssociation"));
    assert!(manifest.contains("ms-resource:AppName"));
    assert_eq!(
        run(&mut Command::new(extracted.join("fixture.exe"))),
        "1.0.0"
    );
}

#[cfg(feature = "native-signing-tests")]
#[test]
fn signed_msix_upgrades_preserve_managed_user_data() {
    let thumbprint = std::env::var("RAKE_TEST_CERT_THUMBPRINT")
        .expect("signing suite requires a disposable local test certificate");

    let trust = TrustOptions {
        windows_thumbprint: Some(thumbprint.clone()),
        ..Default::default()
    };

    let publisher = certificate_subject(&thumbprint).unwrap();
    let root = tempfile::tempdir().unwrap();
    let payload = root.path().join("payload");

    fs::create_dir(&payload).unwrap();
    let icons = rake_build::generate_icons(None, root.path()).unwrap();

    rake_build::generate_msix_assets(&icons, &payload).unwrap();
    let identifier = format!("dev.rake.acceptance.{}", std::process::id());
    let script = root.path().join("install.ps1");

    fs::write(
        &script,
        r#"param([string]$Package,[string]$Name,[string]$Expected)
$ErrorActionPreference='Stop'
Add-AppxPackage -Path $Package
$p=Get-AppxPackage -Name $Name
if ($p.Version.ToString() -ne $Expected) { throw 'wrong installed version' }
$state=Join-Path $env:LOCALAPPDATA ('Packages\'+$p.PackageFamilyName+'\LocalState')
New-Item -ItemType Directory -Force -Path $state | Out-Null
$marker=Join-Path $state 'rake-test-state'
if ($Expected -eq '1.0.0.0') { Set-Content -Path $marker -Value 'preserve me' }
elseif ((Get-Content $marker) -ne 'preserve me') { throw 'upgrade lost user state' }
$p.InstallLocation
"#,
    )
    .unwrap();
    struct Cleanup(String);

    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = Command::new("powershell.exe")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    &format!("Get-AppxPackage -Name '{}' | Remove-AppxPackage", self.0),
                ])
                .status();
        }
    }
    let _cleanup = Cleanup(identifier.clone());

    for (version, expected) in [("1.0.0", "1.0.0.0"), ("1.0.1", "1.0.1.0")] {
        compile(&payload, version);
        sign(&payload.join("fixture.exe"), &trust, None).unwrap();
        let settings = Msix {
            identifier: &identifier,
            publisher: &publisher,
            version: expected,
            target: "x86_64-pc-windows-msvc",
            executable: "fixture.exe",
            name: "Rake Managed Acceptance",
            manifest: None,
        };

        let output = root.path().join(version);

        fs::create_dir(&output).unwrap();
        let package = package_msix(&payload, &output, &settings).unwrap();

        sign(&package, &trust, None).unwrap();
        let location = run(Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-File"])
            .arg(&script)
            .arg(package)
            .arg(&identifier)
            .arg(expected));

        assert_eq!(
            run(&mut Command::new(Path::new(&location).join("fixture.exe"))),
            version
        );
    }
}
