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

    let settings =
        BundleSettings {
            identifier: Some(plan.identifier().into()),
            category: config.desktop.category.map(|category|category.native_name().parse()).transpose().map_err(|error|Error::Execution(format!("native category: {error:?}")))?,
            file_associations: (!config.desktop.file_associations.is_empty()).then(||config.desktop.file_associations.iter().map(|association|serde_json::from_value(serde_json::json!({"ext":association.extensions,"name":association.name,"description":association.description,"mimeType":association.mime_type}))).collect::<Result<_,_>>()).transpose()?,
            deep_link_protocols: (!config.desktop.url_schemes.is_empty()).then(||serde_json::from_value(serde_json::json!({"schemes":config.desktop.url_schemes})).map(|protocol|vec![protocol])).transpose()?,
            publisher: Some(plan.name().into()),
            icon: (!icons.is_empty()).then_some(icons),
            resources_map: Some(resources),
            macos: tauri_bundler::MacOsSettings {
                info_plist: if plan.platform() == crate::Platform::Macos {
                    let mut value = config
                        .macos
                        .info_plist
                        .as_ref()
                        .map(|path| {
                            let path = crate::files::confined(root, path)?;

                            rake_dist_macos::info_plist(
                                &path,
                                plan.identifier(),
                                binary,
                                &item.version().version,
                                config
                                    .build_number
                                    .as_deref()
                                    .unwrap_or(&item.version().version),
                            )
                            .map_err(|error| Error::Execution(error.to_string()))
                        })
                        .transpose()?;

                    if let Some(dictionary) = value.as_ref().and_then(|value|value.as_dictionary()) {
                        for (field, enabled) in [("CFBundleURLTypes",!config.desktop.url_schemes.is_empty()),("CFBundleDocumentTypes",!config.desktop.file_associations.is_empty()),("LSApplicationCategoryType",config.desktop.category.is_some())] {
                            if enabled && dictionary.contains_key(field) {return Err(Error::Execution(format!("{field} is declared by both desktop metadata and the custom plist")));}
                        }
                    }

                    if !config.macos.privacy.is_empty() {
                        let mut plist = value.unwrap_or_else(rake_dist_macos::empty_info_plist);
                        let dictionary = plist.as_dictionary_mut().ok_or_else(||Error::Execution("custom plist must be a dictionary".into()))?;

                        for (key, description) in &config.macos.privacy {
                            if dictionary.get(key).is_some_and(|value|value.as_string()!=Some(description.as_str())) {return Err(Error::Execution(format!("macos.privacy conflicts with custom plist {key}")));}

                            dictionary.insert(key.clone(), description.clone().into());
                        }

                        value=Some(plist);
                    }

                    if item.update_owner() == rake_dist_core::UpdateOwner::Sparkle {
                        let path =
                            config.updates.public_keys.get("sparkle").ok_or_else(|| {
                                Error::Execution("Sparkle public key missing".into())
                            })?;

                        let key = std::fs::read_to_string(crate::files::confined(root, path)?)?;

                        let feed =
                            rake_dist_core::feed_url(
                                config.updates.base_url.as_deref().ok_or_else(|| {
                                    Error::Execution("Sparkle feed missing".into())
                                })?,
                                config.updates.channel.as_deref().unwrap_or("stable"),
                                target,
                                "appcast.xml",
                            )?;

                        value = Some(
                            rake_dist_macos::sparkle_info_plist(
                                value.unwrap_or_else(rake_dist_macos::empty_info_plist),
                                &feed,
                                &key,
                            )
                            .map_err(|error| Error::Execution(error.to_string()))?,
                        );
                    }

                    value.map(tauri_bundler::PlistKind::Plist)
                } else {
                    None
                },
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
                #[expect(deprecated, reason = "pinned 2.10.4 defaults MSI to a project icon; clear it to use the staged common icon")]
                icon_path: PathBuf::new(),
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
