//! Application build-script preparation. Installer engines never enter this dependency graph.
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Prepares portable resource metadata and Windows icon/version resources.
///
/// Call `rake_build::build()?` from the application's build.rs. GPUI owns the
/// linked Win32 manifest; distribution replaces resource #1 on a staged copy
/// when a custom manifest is requested, avoiding duplicate linker resources.
/// # Errors
/// Returns invalid Cargo metadata/icon and native resource compiler errors.
pub fn build() -> Result<(), Box<dyn std::error::Error>> {
    let root =
        PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").ok_or("CARGO_MANIFEST_DIR missing")?);
    let manifest = root.join("Cargo.toml");
    println!("cargo:rerun-if-changed={}", manifest.display());
    let value: toml::Value = toml::from_str(&fs::read_to_string(&manifest)?)?;
    let config = value
        .get("package")
        .and_then(|p| p.get("metadata"))
        .and_then(|p| p.get("rake"))
        .and_then(|p| p.get("bundle"));
    let config: rake_dist_core::Config = config
        .map(|v| serde_json::from_value(serde_json::to_value(v)?))
        .transpose()?
        .unwrap_or_default();
    for variable in [
        "TARGET",
        "RAKE_DISTRIBUTION_FORMAT",
        "RAKE_DISTRIBUTION_TARGET",
        "RAKE_SPARKLE_FRAMEWORK",
    ] {
        println!("cargo:rerun-if-env-changed={variable}");
    }
    let platform = match std::env::var("CARGO_CFG_TARGET_OS")?.as_str() {
        "macos" => Some(rake_dist_core::Platform::Macos),
        "windows" => Some(rake_dist_core::Platform::Windows),
        "linux" => Some(rake_dist_core::Platform::Linux),
        _ => None,
    };
    let config = if let Some(platform) = platform {
        let format = std::env::var("RAKE_DISTRIBUTION_FORMAT")
            .ok()
            .map(|s| serde_json::from_value::<rake_dist_core::Format>(serde_json::Value::String(s)))
            .transpose()?;
        let selected = format.into_iter().collect::<Vec<_>>();
        let distribution_target =
            std::env::var("RAKE_DISTRIBUTION_TARGET").unwrap_or(std::env::var("TARGET")?);
        let plan = config.plan_for(
            &std::env::var("CARGO_PKG_NAME")?,
            &std::env::var("CARGO_PKG_VERSION")?,
            platform,
            Some(&distribution_target),
            (!selected.is_empty()).then_some(selected.as_slice()),
        )?;
        let package = plan.packages().first().ok_or("no selected package")?;
        let mut public_keys = std::collections::BTreeMap::new();
        for (provider, path) in &package.config().updates.public_keys {
            println!("cargo:rerun-if-changed={}", root.join(path).display());
            let bytes = fs::read(root.join(path))?;
            if bytes.is_empty() || bytes.len() > 16 * 1024 {
                return Err("update public keys must contain 1–16384 bytes".into());
            }
            public_keys.insert(provider.clone(), bytes);
        }
        let binding = rake_dist_core::UpdateBinding {
            schema_version: 1,
            identifier: plan.identifier().into(),
            version: plan.version().into(),
            package_version: package.version().version.clone(),
            target: distribution_target,
            format: package.format(),
            owner: package.update_owner(),
            channel: package
                .config()
                .updates
                .channel
                .clone()
                .unwrap_or_else(|| "stable".into()),
            base_url: package.config().updates.base_url.clone(),
            public_keys,
        };
        binding.validate()?;
        println!(
            "cargo:rustc-env=RAKE_UPDATE_POLICY={}",
            serde_json::to_string(&binding)?
        );
        if package.update_owner() == rake_dist_core::UpdateOwner::Sparkle {
            println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
        }
        package.config().clone()
    } else {
        config
    };
    let name = config
        .name
        .as_deref()
        .map(str::to_owned)
        .unwrap_or(std::env::var("CARGO_PKG_NAME")?);
    let resource_directory = if platform == Some(rake_dist_core::Platform::Linux) {
        config.linux.package_name.as_deref().unwrap_or(&name)
    } else {
        &name
    };
    println!("cargo:rustc-env=RAKE_RESOURCE_DIRECTORY={resource_directory}");
    println!("cargo:rustc-env=RAKE_BUILD_RESOURCES=1");
    for path in [
        config.icon.as_ref(),
        config.windows.manifest.as_ref(),
        config.macos.info_plist.as_ref(),
        config.macos.entitlements.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        rake_dist_core::validate_relative_path(path)?;
        println!("cargo:rerun-if-changed={}", root.join(path).display());
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let output = PathBuf::from(std::env::var_os("OUT_DIR").ok_or("OUT_DIR missing")?);
        let icon = generate_icons(
            config.icon.as_ref().map(|p| root.join(p)).as_deref(),
            &output,
        )?;
        let mut resource = tauri_winres::WindowsResource::new();
        let version = config
            .package_version
            .clone()
            .unwrap_or(std::env::var("CARGO_PKG_VERSION")?);
        let numeric = windows_version(&version)?;
        resource
            .set_version_info(tauri_winres::VersionInfo::FILEVERSION, numeric)
            .set_version_info(tauri_winres::VersionInfo::PRODUCTVERSION, numeric)
            .set("FileVersion", &version)
            .set("ProductVersion", &version);
        resource
            .set("ProductName", &name)
            .set("FileDescription", &name)
            .set(
                "OriginalFilename",
                &format!("{}.exe", std::env::var("CARGO_PKG_NAME")?),
            );
        resource.set_icon(
            icon.join("icon.ico")
                .to_str()
                .ok_or("icon path must be UTF-8")?,
        );
        resource.compile()?;
    }
    Ok(())
}

fn windows_version(version: &str) -> Result<u64, Box<dyn std::error::Error>> {
    let version = version
        .split(['-', '+'])
        .next()
        .ok_or("missing Windows file version")?;
    let fields: Vec<u16> = version
        .split('.')
        .map(str::parse)
        .collect::<Result<_, _>>()?;
    if !(3..=4).contains(&fields.len()) {
        return Err("Windows file version needs three or four numeric fields".into());
    }
    Ok(fields
        .iter()
        .chain(std::iter::repeat(&0))
        .take(4)
        .fold(0_u64, |value, field| (value << 16) | u64::from(*field)))
}

/// Generates PNG and ICO variants from one PNG; a simple neutral icon is the default.
/// # Errors
/// Returns decoding, dimension or filesystem errors. SVG rasterization disables external resource loading.
pub fn generate_icons(
    source: Option<&Path>,
    output: &Path,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let image = match source {
        Some(path) if path.extension().is_some_and(|extension| extension == "svg") => {
            let mut options = resvg::usvg::Options::default();
            options.image_href_resolver.resolve_string = Box::new(|_, _| None);
            options.image_href_resolver.resolve_data = Box::new(|_, _, _| None);
            let bytes = fs::read(path)?;
            if bytes.len() > 4 * 1024 * 1024 {
                return Err("SVG icon exceeds 4 MiB".into());
            }
            let tree = resvg::usvg::Tree::from_data(&bytes, &options)?;
            let mut pixels =
                resvg::tiny_skia::Pixmap::new(256, 256).ok_or("icon allocation failed")?;
            let scale = resvg::tiny_skia::Transform::from_scale(
                256.0 / tree.size().width(),
                256.0 / tree.size().height(),
            );
            resvg::render(&tree, scale, &mut pixels.as_mut());
            // tiny-skia stores premultiplied RGBA; PNG encoding unpremultiplies.
            image::load_from_memory(&pixels.encode_png()?)?.into_rgba8()
        }
        Some(path) => {
            let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(8192);
            limits.max_image_height = Some(8192);
            limits.max_alloc = Some(128 * 1024 * 1024);
            reader.limits(limits);
            reader.decode()?.into_rgba8()
        }
        None => image::RgbaImage::from_pixel(256, 256, image::Rgba([42, 90, 70, 255])),
    };
    if image.width() != image.height() || image.width() < 32 || image.width() > 8192 {
        return Err("icon must be square, 32–8192 pixels".into());
    }
    let directory = output.join("icons");
    fs::create_dir_all(&directory)?;
    let resized = image::imageops::resize(&image, 256, 256, image::imageops::FilterType::Lanczos3);
    resized.save(directory.join("icon.png"))?;
    resized.save(directory.join("icon.ico"))?;
    Ok(directory)
}

/// Generates required MSIX store/tile assets from the normalized icon.
/// # Errors
/// Returns icon decoding or output filesystem errors.
pub fn generate_msix_assets(
    icons: &Path,
    payload: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let image = image::open(icons.join("icon.png"))?.into_rgba8();
    let assets = payload.join("Assets");
    fs::create_dir_all(&assets)?;
    for (name, size) in [
        ("StoreLogo.png", 50),
        ("Square150x150Logo.png", 150),
        ("Square44x44Logo.png", 44),
    ] {
        image::imageops::resize(&image, size, size, image::imageops::FilterType::Lanczos3)
            .save(assets.join(name))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn pe_versions_preserve_numeric_mapping_and_reject_overflow() {
        assert_eq!(
            super::windows_version("1.2.3-beta.4").unwrap(),
            (1_u64 << 48) | (2_u64 << 32) | (3_u64 << 16)
        );
        assert_eq!(
            super::windows_version("1.2.3.4").unwrap(),
            (1_u64 << 48) | (2_u64 << 32) | (3_u64 << 16) | 4
        );
        assert!(super::windows_version("65536.1.0").is_err());
    }
}
