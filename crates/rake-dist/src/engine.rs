//! The only production boundary that imports upstream bundler types.
use crate::{Error, Format, Plan};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

pub(crate) struct Input<'a> {
    pub plan: &'a Plan,
    pub index: usize,
    pub bin: &'a Path,
    pub binary: &'a str,
    pub target: &'a str,
    pub root: &'a Path,
    pub resources: HashMap<String, String>,
    pub icons: Vec<String>,
    pub helpers: Vec<String>,
    pub frameworks: Vec<PathBuf>,
    pub linux_files: HashMap<PathBuf, PathBuf>,
}

pub(crate) fn package(input: Input<'_>) -> Result<Vec<PathBuf>, Error> {
    let Input {
        plan,
        index,
        bin,
        binary,
        target,
        root,
        resources,
        icons,
        helpers,
        frameworks,
        linux_files,
    } = input;
    use tauri_bundler::{
        BundleBinary, BundleSettings, PackageSettings, PackageType, SettingsBuilder,
    };
    let item = &plan.packages()[index];
    let kind = match item.format() {
        Format::App => PackageType::MacOsBundle,
        Format::Dmg => PackageType::MacOsBundle,
        Format::Zip if plan.platform() == crate::Platform::Macos => PackageType::MacOsBundle,
        Format::Nsis => PackageType::Nsis,
        Format::Msi => PackageType::WindowsMsi,
        Format::Appimage => PackageType::AppImage,
        other => {
            return Err(Error::Execution(format!(
                "format {other:?} requires a dedicated backend"
            )));
        }
    };
    let config = item.config();
    // Upstream resource normalization drops Windows drive prefixes. Inputs are
    // staged in this worker, so relative paths avoid cross-drive ambiguity.
    let resources = resources
        .into_iter()
        .map(|(source, destination)| Ok((worker_relative(Path::new(&source))?, destination)))
        .collect::<Result<HashMap<_, _>, Error>>()?;
    let icons = icons
        .iter()
        .map(|path| worker_relative(Path::new(path)))
        .collect::<Result<Vec<_>, _>>()?;
    let settings = BundleSettings {
        identifier: Some(plan.identifier().into()),
        publisher: Some(plan.name().into()),
        icon: (!icons.is_empty()).then_some(icons),
        resources_map: Some(resources),
        macos: tauri_bundler::MacOsSettings {
            info_plist: config
                .macos
                .info_plist
                .as_ref()
                .filter(|_| plan.platform() == crate::Platform::Macos)
                .map(|path| {
                    let path = crate::files::confined(root, path)?;
                    let value = rake_dist_macos::info_plist(
                        &path,
                        plan.identifier(),
                        binary,
                        &item.version().version,
                        config
                            .build_number
                            .as_deref()
                            .unwrap_or(&item.version().version),
                    )
                    .map_err(|e| Error::Execution(e.to_string()))?;
                    Ok::<_, Error>(tauri_bundler::PlistKind::Plist(value))
                })
                .transpose()?,
            bundle_version: config.build_number.clone(),
            frameworks: (!frameworks.is_empty()).then(|| {
                frameworks
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect()
            }),
            minimum_system_version: config.minimum_system_version.clone(),
            ..Default::default()
        },
        appimage: tauri_bundler::AppImageSettings {
            files: linux_files,
            ..Default::default()
        },
        windows: tauri_bundler::WindowsSettings {
            webview_install_mode: serde_json::from_value(serde_json::json!({"type":"skip"}))?,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut binaries = vec![BundleBinary::new(binary.into(), true)];
    binaries.extend(
        helpers
            .into_iter()
            .map(|name| BundleBinary::new(name, false)),
    );
    let settings = SettingsBuilder::new()
        .project_out_directory(bin)
        .package_types(vec![kind])
        .package_settings(PackageSettings {
            product_name: if plan.platform() == crate::Platform::Linux {
                config.linux.package_name.as_deref().unwrap_or(plan.name())
            } else {
                config.name.as_deref().unwrap_or(plan.name())
            }
            .into(),
            version: item.version().version.clone(),
            description: format!("{} desktop application", plan.name()),
            homepage: None,
            authors: Some(vec![plan.name().into()]),
            default_run: Some(binary.into()),
        })
        .bundle_settings(settings)
        .binaries(binaries)
        .target(target.into())
        .binary_patching(false)
        .no_sign(true)
        .build()
        .map_err(|e| Error::Execution(format!("bundler settings: {e}")))?;
    let packages = tauri_bundler::bundle_project(&settings)
        .map_err(|e| Error::Execution(format!("bundler: {e}")))?;
    if !packages.iter().any(|p| p.package_type == kind) {
        return Err(Error::Execution("engine skipped requested format".into()));
    }
    Ok(packages
        .into_iter()
        .filter(|p| p.package_type == kind)
        .flat_map(|p| p.bundle_paths)
        .collect())
}

fn worker_relative(path: &Path) -> Result<String, Error> {
    let current = dunce::canonicalize(std::env::current_dir()?)?;
    let path = dunce::canonicalize(path)?;
    let relative = path.strip_prefix(&current).map_err(|_| {
        Error::Execution("bundler inputs must stay inside the worker workspace".into())
    })?;
    Ok(rake_dist_core::portable_relative_path(relative)?
        .to_string_lossy()
        .into_owned())
}
