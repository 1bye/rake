//! Linux installation layout and SDK source preparation owned by the coordinator.
use crate::{Error, Format, Metadata, Package, components, files};
use rake_dist_core::{Config, Plan};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub(crate) fn dependencies(config: &Config, format: Format) -> Vec<String> {
    let mut result = config
        .linux
        .dependencies
        .get(&format)
        .cloned()
        .unwrap_or_default();
    for runtime in config.runtimes.iter().filter(|runtime| runtime.system) {
        result.extend(
            runtime
                .system_dependencies
                .get(&format)
                .cloned()
                .unwrap_or_default(),
        );
    }
    result.sort();
    result.dedup();
    result
}

pub(crate) fn integration(
    plan: &Plan,
    index: usize,
    binary: &str,
    root: &Path,
    stage: &Path,
    icons: &Path,
) -> Result<HashMap<PathBuf, PathBuf>, Error> {
    let config = plan.packages()[index].config();
    let flatpak = plan.packages()[index].format() == Format::Flatpak;
    let desktop_id = if flatpak {
        plan.identifier()
    } else {
        config.linux.package_name.as_deref().unwrap_or(binary)
    };
    let directory = stage.join("linux-metadata");
    fs::create_dir_all(&directory)?;
    let desktop = directory.join(format!("{desktop_id}.desktop"));
    if config.linux.desktop_entry.is_some() && !config.desktop.is_empty() {
        return Err(Error::Execution("custom linux.desktop_entry owns its labels and registrations; use either that file or desktop metadata".into()));
    }
    if let Some(source) = &config.linux.desktop_entry {
        files::copy(&files::confined(root, source)?, &desktop)?;
        let text = fs::read_to_string(&desktop)?;
        if !text
            .lines()
            .any(|line| line.starts_with(&format!("Exec={binary}")))
        {
            return Err(Error::Execution(
                "desktop entry must launch the selected binary".into(),
            ));
        }
    } else {
        fs::write(
            &desktop,
            rake_dist_linux::desktop_entry_with_metadata(
                config.name.as_deref().unwrap_or(plan.name()),
                binary,
                desktop_id,
                &config.desktop,
            )
            .map_err(native)?,
        )?;
    }
    let appstream = directory.join(format!("{}.metainfo.xml", plan.identifier()));
    let text = if let Some(source) = &config.linux.appstream {
        fs::read_to_string(files::confined(root, source)?)?
    } else {
        rake_dist_linux::appstream_with_metadata(
            plan.identifier(),
            config.name.as_deref().unwrap_or(plan.name()),
            desktop_id,
            plan.version(),
            &config.desktop,
        )
        .map_err(native)?
    };
    rake_dist_linux::validate_appstream(&text, plan.identifier(), desktop_id).map_err(native)?;
    fs::write(&appstream, text)?;
    let mut files = HashMap::from([
        (
            PathBuf::from(format!("usr/share/applications/{desktop_id}.desktop")),
            desktop,
        ),
        (
            PathBuf::from(format!(
                "usr/share/metainfo/{}.metainfo.xml",
                plan.identifier()
            )),
            appstream,
        ),
        (
            PathBuf::from(format!(
                "usr/share/icons/hicolor/256x256/apps/{desktop_id}.png"
            )),
            icons.join("icon.png"),
        ),
    ]);
    if !config.desktop.file_associations.is_empty() {
        let mime = directory.join("mime-types.xml");
        fs::write(
            &mime,
            rake_dist_linux::mime_types(desktop_id, &config.desktop).map_err(native)?,
        )?;
        files.insert(
            PathBuf::from(format!("usr/share/mime/packages/{desktop_id}.xml")),
            mime,
        );
    }
    Ok(files)
}

pub(crate) fn payload(
    stage: &Path,
    binary: &str,
    bin: &Path,
    resources: &Path,
    name: &str,
    metadata: &HashMap<PathBuf, PathBuf>,
    include_binary: bool,
) -> Result<PathBuf, Error> {
    let payload = stage.join("linux-payload");
    fs::create_dir_all(&payload)?;
    if include_binary {
        files::copy(&bin.join(binary), &payload.join("usr/bin").join(binary))?;
    }
    files::copy(resources, &payload.join("usr/lib").join(name))?;
    for (destination, source) in metadata {
        files::copy(source, &payload.join(destination))?;
    }
    Ok(payload)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn flatpak(
    plan: &Plan,
    index: usize,
    binary: &str,
    selected: &Package,
    metadata: &Metadata,
    target: &str,
    payload: &Path,
    stage: &Path,
    offline: bool,
    features: &[String],
) -> Result<Vec<PathBuf>, Error> {
    let config = plan.packages()[index].config();
    let output = stage.join("flatpak");
    fs::create_dir_all(&output)?;
    let manifest = output.join("manifest.json");
    if let Some(custom) = &config.linux.flatpak.manifest {
        // Keep source paths relative to the original override directory.
        let path = files::confined(
            selected.manifest_path.parent().ok_or(Error::Package)?,
            custom,
        )?;
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
        let path_parent = path.parent().ok_or(Error::Package)?;
        let mut value = value;
        absolute_sources(&mut value, path_parent)?;
        files::write_json(&manifest, &value)?;
    } else {
        let source = output.join("source");
        snapshot(&metadata.workspace_root, &source)?;
        let relative = selected
            .manifest_path
            .strip_prefix(&metadata.workspace_root)
            .map_err(|_| {
                Error::Execution("Flatpak package must belong to its Cargo workspace".into())
            })?;
        vendor(&selected.manifest_path, &source)?;
        let profile: serde_json::Value =
            serde_json::from_str(include_str!("flatpak-rust-tools.json"))?;
        let values = profile
            .get(target)
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| {
                Error::Execution(
                    "no pinned Flatpak compiler for this target; provide a custom manifest".into(),
                )
            })?;
        let cache = metadata.target_directory.join("rake-component-cache");
        fs::create_dir_all(&cache)?;
        let mut tools = Vec::new();
        for (name, value) in values {
            let url = value["url"]
                .as_str()
                .ok_or_else(|| Error::Execution("invalid compiler URL profile".into()))?;
            let sha256 = value["sha256"]
                .as_str()
                .ok_or_else(|| Error::Execution("invalid compiler SHA profile".into()))?;
            let archive = components::download(url, sha256, 512 * 1024 * 1024, &cache, offline)?;
            tools.push(rake_dist_linux::FlatpakTool {
                name: name.clone(),
                archive,
                sha256: sha256.into(),
            });
        }
        let value = rake_dist_linux::flatpak_manifest(
            plan.identifier(),
            binary,
            &selected.name,
            relative,
            &source,
            payload,
            &config.linux.flatpak,
            &tools,
            features,
        )
        .map_err(native)?;
        files::write_json(&manifest, &value)?;
    }
    let (artifact, records) = rake_dist_linux::build_flatpak(&rake_dist_linux::FlatpakBuild {
        identifier: plan.identifier(),
        target,
        config: &config.linux.flatpak,
        manifest: &manifest,
        output: &output,
    })
    .map_err(native)?;
    let report = output.join("toolchain.json");
    files::write_json(&report, &records)?;
    Ok(vec![artifact, manifest, report])
}
fn native(error: rake_dist_linux::Error) -> Error {
    Error::Execution(error.to_string())
}
fn absolute_sources(value: &mut serde_json::Value, root: &Path) -> Result<(), Error> {
    match value {
        serde_json::Value::Object(object) => {
            if object.contains_key("type")
                && let Some(path) = object.get_mut("path")
                && let Some(relative) = path.as_str()
            {
                *path = serde_json::to_value(files::confined(root, Path::new(relative))?)?;
            }
            for child in object.values_mut() {
                absolute_sources(child, root)?;
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                absolute_sources(child, root)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn snapshot(root: &Path, destination: &Path) -> Result<(), Error> {
    fs::create_dir_all(destination)?;
    let output = Command::new("git")
        .current_dir(root)
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .output()?;
    let paths = if output.status.success() {
        output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|bytes| !bytes.is_empty())
            .map(|bytes| {
                String::from_utf8(bytes.to_vec())
                    .map(PathBuf::from)
                    .map_err(|_| Error::Execution("Flatpak source paths must be UTF-8".into()))
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        source_files(root, root)?
    };
    let mut count = 0;
    for path in paths {
        if path.components().any(|component| {
            let name = component.as_os_str().to_string_lossy();
            matches!(
                name.as_ref(),
                "target" | ".git" | ".agents" | ".codex" | ".env"
            ) || name.starts_with(".env.")
        }) {
            continue;
        }
        if root.join(&path).is_file() || root.join(&path).is_symlink() {
            count += 1;
            if count > 100_000 {
                return Err(Error::Execution(
                    "Flatpak workspace exceeds source file bound".into(),
                ));
            }
            files::copy(&files::confined(root, &path)?, &destination.join(path))?;
        }
    }
    Ok(())
}
fn source_files(root: &Path, directory: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut output = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        if matches!(
            name.to_str(),
            Some("target" | ".git" | ".agents" | ".codex")
        ) {
            continue;
        }
        if entry.file_type()?.is_dir() {
            output.extend(source_files(root, &entry.path())?);
        } else {
            output.push(
                entry
                    .path()
                    .strip_prefix(root)
                    .map_err(|_| Error::Execution("source escaped workspace".into()))?
                    .to_owned(),
            );
        }
    }
    Ok(output)
}
fn vendor(manifest: &Path, source: &Path) -> Result<(), Error> {
    let output = Command::new("cargo")
        .current_dir(manifest.parent().ok_or(Error::Package)?)
        .args(["vendor", "--locked", "--versioned-dirs", "--manifest-path"])
        .arg(manifest)
        .arg(source.join("vendor"))
        .output()?;
    if !output.status.success() {
        return Err(Error::Cargo(String::from_utf8_lossy(&output.stderr).into()));
    }
    let vendor: toml::Value = toml::from_str(&String::from_utf8_lossy(&output.stdout))
        .map_err(|error| Error::Execution(error.to_string()))?;
    let directory = source.join(".cargo");
    fs::create_dir_all(&directory)?;
    let config = directory.join("config.toml");
    let existing = if config.exists() {
        toml::from_str::<toml::Value>(&fs::read_to_string(&config)?)
            .map_err(|error| Error::Execution(error.to_string()))?
    } else {
        toml::Value::Table(Default::default())
    };
    let existing = vendor_configuration(existing, vendor)?;
    fs::write(
        config,
        toml::to_string(&existing).map_err(|error| Error::Execution(error.to_string()))?,
    )?;
    Ok(())
}

fn vendor_configuration(
    mut existing: toml::Value,
    mut vendor: toml::Value,
) -> Result<toml::Value, Error> {
    if let Some(table) = vendor.get_mut("source").and_then(toml::Value::as_table_mut) {
        for (_, value) in table.iter_mut() {
            if let Some(directory) = value.get_mut("directory") {
                *directory = toml::Value::String("vendor".into());
            }
        }
    }
    if let Some(sources) = vendor.get("source") {
        existing
            .as_table_mut()
            .ok_or_else(|| Error::Execution("Cargo config must be a table".into()))?
            .insert("source".into(), sources.clone());
    }
    Ok(existing)
}
#[cfg(test)]
mod vendor_tests {
    #[test]
    fn vendor_sources_are_inserted_when_the_workspace_has_no_source_table() {
        let vendor:toml::Value=toml::from_str("[source.crates-io]\nreplace-with='vendored-sources'\n[source.vendored-sources]\ndirectory='/temporary/host/path/vendor'\n").unwrap();
        let existing: toml::Value = toml::from_str("[build]\njobs=2\n").unwrap();
        let value = super::vendor_configuration(existing, vendor).unwrap();
        assert_eq!(value["build"]["jobs"].as_integer(), Some(2));
        assert_eq!(
            value["source"]["vendored-sources"]["directory"].as_str(),
            Some("vendor")
        );
    }
}
