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
    for variable in ["TARGET", "RAKE_DISTRIBUTION_FORMAT"] {
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
        let plan = config.plan_for(
            &std::env::var("CARGO_PKG_NAME")?,
            &std::env::var("CARGO_PKG_VERSION")?,
            platform,
            Some(&std::env::var("TARGET")?),
            (!selected.is_empty()).then_some(selected.as_slice()),
        )?;
        plan.packages()
            .first()
            .ok_or("no selected package")?
            .config()
            .clone()
    } else {
        config
    };
    let name = config
        .name
        .as_deref()
        .map(str::to_owned)
        .unwrap_or(std::env::var("CARGO_PKG_NAME")?);
    println!("cargo:rustc-env=RAKE_RESOURCE_DIRECTORY={name}");
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
