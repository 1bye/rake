//! Built-in native deployment recipes. Loaders use only declared component roots.
use crate::{Error, components, files};
use rake_dist_core::{Config, Installation, Platform, Runtime};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn root<'a>(
    component: Option<&str>,
    roots: &'a BTreeMap<String, PathBuf>,
    package: &'a Path,
) -> Result<&'a Path, Error> {
    match component {
        Some(name) => roots.get(name).map(PathBuf::as_path).ok_or_else(|| {
            Error::Execution(format!(
                "component {name} has no source for the selected target"
            ))
        }),
        None => Ok(package),
    }
}

pub(crate) struct Input<'a> {
    pub config: &'a Config,
    pub package: &'a Path,
    pub roots: &'a BTreeMap<String, PathBuf>,
    pub resource_stage: &'a Path,
    pub target: &'a str,
    pub descriptor: &'a mut Installation,
    pub resources: &'a mut HashMap<String, String>,
}

pub(crate) fn runtimes(input: Input<'_>) -> Result<Vec<PathBuf>, Error> {
    let Input {
        config,
        package,
        roots,
        resource_stage,
        target,
        descriptor,
        resources,
    } = input;
    let mut frameworks = Vec::new();
    for runtime in &config.runtimes {
        let source = files::confined(
            root(runtime.component.as_deref(), roots, package)?,
            &runtime.source,
        )?;
        match runtime.recipe.as_str() {
            "framework" if descriptor.platform == Platform::Macos => {
                let name = source
                    .file_name()
                    .ok_or_else(|| Error::Execution("framework name".into()))?;
                if source
                    .extension()
                    .is_none_or(|extension| extension != "framework")
                {
                    return Err(Error::Execution(
                        "framework recipe requires a .framework directory".into(),
                    ));
                }
                let stage = resource_stage
                    .parent()
                    .ok_or_else(|| Error::Execution("runtime stage".into()))?
                    .join("frameworks")
                    .join(name);
                files::copy(&source, &stage)?;
                let stem = source
                    .file_stem()
                    .ok_or_else(|| Error::Execution("framework stem".into()))?;
                components::architecture(&stage.join(stem), target)?;
                if !runtime.plugins.is_empty() {
                    select_gstreamer_plugins(runtime, &stage.join("Versions/Current"))?;
                }
                inspect_tree(&stage, target)?;
                descriptor
                    .frameworks
                    .insert(runtime.name.clone(), PathBuf::from(name));
                if let Some(component) = &runtime.component {
                    descriptor
                        .native_groups
                        .entry(component.clone())
                        .or_default()
                        .push(PathBuf::from("Frameworks").join(name));
                }
                frameworks.push(stage);
            }
            "directory" | "gstreamer" => {
                let relative = PathBuf::from("rake/runtimes").join(&runtime.name);
                let destination = resource_stage.join(&relative);
                files::copy(&source, &destination)?;
                if runtime.recipe == "gstreamer" {
                    select_gstreamer_plugins(runtime, &destination)?;
                }
                inspect_tree(&destination, target)?;
                resources.insert(
                    destination.to_string_lossy().into_owned(),
                    relative.to_string_lossy().into_owned(),
                );
                if let Some(component) = &runtime.component {
                    let member = if descriptor.platform == Platform::Macos {
                        PathBuf::from("Resources").join(&relative)
                    } else {
                        relative.clone()
                    };
                    descriptor
                        .native_groups
                        .entry(component.clone())
                        .or_default()
                        .push(member);
                }
                descriptor.runtimes.insert(runtime.name.clone(), relative);
            }
            name => {
                return Err(Error::Execution(format!(
                    "unknown or incompatible runtime recipe: {name}"
                )));
            }
        }
    }
    Ok(frameworks)
}

fn inspect_tree(path: &Path, target: &str) -> Result<(), Error> {
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            if !entry.file_type()?.is_symlink() {
                inspect_tree(&entry.path(), target)?;
            }
        }
    } else {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if name.ends_with(".dll")
            || name.ends_with(".dylib")
            || name.contains(".so")
            || name.ends_with(".exe")
        {
            components::architecture(path, target)?;
        }
    }
    Ok(())
}

fn select_gstreamer_plugins(runtime: &Runtime, directory: &Path) -> Result<(), Error> {
    if runtime.plugins.is_empty() {
        return Err(Error::Execution(
            "gstreamer requires an explicit plugin selection".into(),
        ));
    }
    let plugins = directory.join("lib/gstreamer-1.0");
    if !plugins.is_dir() {
        return Err(Error::Execution(
            "gstreamer runtime requires lib/gstreamer-1.0".into(),
        ));
    }
    let mut found = std::collections::BTreeSet::new();
    for entry in fs::read_dir(&plugins)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let selected = runtime.plugins.iter().find(|plugin| {
            name == **plugin
                || name.starts_with(&format!("libgst{plugin}."))
                || name.starts_with(&format!("gst{plugin}."))
        });
        if let Some(plugin) = selected {
            found.insert(plugin.clone());
        } else if entry.file_type()?.is_file() || entry.file_type()?.is_symlink() {
            fs::remove_file(entry.path())?;
        } else {
            return Err(Error::Execution(
                "unexpected GStreamer plugin directory".into(),
            ));
        }
    }
    for plugin in &runtime.plugins {
        if !found.contains(plugin) {
            return Err(Error::Execution(format!(
                "missing selected GStreamer plugin {plugin}"
            )));
        }
    }
    Ok(())
}
