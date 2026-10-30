//! Isolated executable-packager adoption fixture; no Rake/GPUI runtime dependency.

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let root = PathBuf::from(args.next().ok_or("expected absolute fixture directory")?);
    let format = args.next().ok_or("expected package format")?;
    let format = format.to_str().ok_or("format must be UTF-8")?;
    let target = args.next().ok_or("expected Rust target triple")?;
    let target = target.to_str().ok_or("target must be UTF-8")?;
    if !root.is_absolute() || args.next().is_some() {
        return Err("expected DIRECTORY FORMAT TARGET".into());
    }

    #[cfg(feature = "cargo-packager")]
    let paths = cargo_package(&root, format, target)?;
    #[cfg(feature = "tauri-bundler")]
    let paths = tauri_package(&root, format, target)?;
    #[cfg(not(any(feature = "cargo-packager", feature = "tauri-bundler")))]
    compile_error!("select one engine feature");
    #[cfg(all(feature = "cargo-packager", feature = "tauri-bundler"))]
    compile_error!("select only one engine feature");

    println!("{}", serde_json::to_string(&paths)?);
    Ok(())
}

#[cfg(feature = "cargo-packager")]
fn cargo_package(
    root: &std::path::Path,
    format: &str,
    target: &str,
) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    use cargo_packager::{Config, PackageFormat};
    let format = PackageFormat::from_short_name(format).ok_or("unknown cargo-packager format")?;
    let mut config: Config = serde_json::from_value(serde_json::json!({
        "productName": "Adoption Fixture", "version": "1.0.0",
        "identifier": "dev.rake.adoption", "publisher": "Rake",
        "description": "Native packaging adoption fixture",
        "authors": ["Rake <test@example.com>"], "outDir": root.join("packages"),
        "binariesDir": root.join("bin"), "targetTriple": target,
        "binaries": [{"path": root.join("bin").join("fixture"), "main": true}, {"path": root.join("bin").join("fixture-helper")}],
        "resources": [root.join("assets").join("message.txt")], "icons": [root.join("icon.png"), root.join("icon.ico")],
        "formats": [format]
    }))?;
    if cfg!(target_os = "macos") {
        config.macos = Some(serde_json::from_value(serde_json::json!({
            "frameworks": [root.join("FixtureRuntime.framework")],
            "infoPlistPath": root.join("extra.plist")
        }))?);
    }
    let packages = cargo_packager::package(&config)?;
    if packages.is_empty() {
        return Err("engine silently skipped requested format".into());
    }
    Ok(packages
        .into_iter()
        .flat_map(|package| package.paths)
        .collect())
}

#[cfg(feature = "tauri-bundler")]
fn tauri_package(
    root: &std::path::Path,
    format: &str,
    target: &str,
) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    use tauri_bundler::{
        BundleBinary, BundleSettings, PackageSettings, PackageType, SettingsBuilder,
    };
    let bundle = BundleSettings {
        identifier: Some("dev.rake.adoption".into()),
        publisher: Some("Rake".into()),
        icon: Some(vec![
            root.join("icon.png").to_string_lossy().into_owned(),
            root.join("icon.ico").to_string_lossy().into_owned(),
        ]),
        resources_map: Some(std::collections::HashMap::from([(
            root.join("assets")
                .join("message.txt")
                .to_string_lossy()
                .into_owned(),
            "assets/message.txt".into(),
        )])),
        macos: tauri_bundler::MacOsSettings {
            frameworks: if cfg!(target_os = "macos") {
                Some(vec![
                    root.join("FixtureRuntime.framework")
                        .to_string_lossy()
                        .into_owned(),
                ])
            } else {
                None
            },
            info_plist: Some(tauri_bundler::PlistKind::Path(root.join("extra.plist"))),
            ..Default::default()
        },
        windows: tauri_bundler::WindowsSettings {
            webview_install_mode: serde_json::from_value(serde_json::json!({"type": "skip"}))?,
            ..Default::default()
        },
        ..Default::default()
    };
    let settings = SettingsBuilder::new()
        .project_out_directory(root.join("bin"))
        .package_types(vec![
            PackageType::from_short_name(format).ok_or("unknown Tauri format")?,
        ])
        .package_settings(PackageSettings {
            product_name: "Adoption Fixture".into(),
            version: "1.0.0".into(),
            description: "Native packaging adoption fixture".into(),
            homepage: None,
            authors: Some(vec!["Rake <test@example.com>".into()]),
            default_run: Some("fixture".into()),
        })
        .bundle_settings(bundle)
        .binaries(vec![
            BundleBinary::new("fixture".into(), true),
            BundleBinary::new("fixture-helper".into(), false),
        ])
        .target(target.into())
        .binary_patching(false)
        .no_sign(true)
        .build()?;
    let packages = tauri_bundler::bundle_project(&settings)?;
    if packages.is_empty() {
        return Err("engine skipped requested format".into());
    }
    Ok(packages
        .into_iter()
        .flat_map(|package| package.bundle_paths)
        .collect())
}
