//! Legacy native JSON/layout adapter; packaging and signing stay in the native coordinator.
use crate::{Artifact, Error, Plan, Platform, files};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub(crate) fn owns(output: &Path, package: &str) -> bool {
    let marker = output.join(".rake-build.json");
    !output.is_symlink()
        && !marker.is_symlink()
        && fs::read(marker)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            == Some(serde_json::json!({"package":package,"target":"desktop","command":"bundle"}))
}
pub(crate) fn mac_icon(app: &Path) -> Result<(), Error> {
    let path = app.join("Contents/Info.plist");
    let mut info =
        plist::Value::from_file(&path).map_err(|error| Error::Execution(error.to_string()))?;
    let dict = info
        .as_dictionary_mut()
        .ok_or_else(|| Error::Execution("invalid generated plist".into()))?;
    if let Some(icon) = dict
        .get("CFBundleIconFile")
        .and_then(plist::Value::as_string)
        .map(str::to_owned)
    {
        rake_dist_core::validate_relative_path(Path::new(&icon))?;
        let original = app.join("Contents/Resources").join(&icon);
        let original = if original.is_file() {
            original
        } else {
            original.with_extension("icns")
        };
        if original.is_file() {
            let destination = app.join("Contents/Resources/rake-icon.icns");
            if original != destination {
                fs::rename(original, destination)?;
            }
            dict.insert("CFBundleIconFile".into(), "rake-icon.icns".into());
            info.to_file_xml(path)
                .map_err(|error| Error::Execution(error.to_string()))?;
        }
    }
    Ok(())
}
fn tool(args: &[&str], file: &Path) -> Result<String, Error> {
    let result = Command::new("otool").args(args).arg(file).output()?;
    if !result.status.success() {
        return Err(Error::Execution(
            String::from_utf8_lossy(&result.stderr).into(),
        ));
    }
    Ok(String::from_utf8_lossy(&result.stdout).into())
}
/// Legacy manifests implicitly bundled linked plain dylibs. Only that compatibility
/// mode resolves load commands; schema-1 dependencies remain explicitly declared.
pub(crate) fn mac_libraries(binary: &Path, stage: &Path) -> Result<Vec<PathBuf>, Error> {
    let executable = fs::canonicalize(binary)?;
    let mut queue = vec![(executable.clone(), Vec::<PathBuf>::new())];
    let mut seen = BTreeSet::new();
    let mut names = BTreeMap::new();
    let expand = |value: &str, loader: &Path| {
        PathBuf::from(
            value
                .replace(
                    "@loader_path",
                    &loader.parent().unwrap_or(loader).to_string_lossy(),
                )
                .replace(
                    "@executable_path",
                    &executable.parent().unwrap_or(&executable).to_string_lossy(),
                ),
        )
    };
    while let Some((file, inherited)) = queue.pop() {
        if !seen.insert(file.clone()) {
            continue;
        }
        if seen.len() > 4096 {
            return Err(Error::Execution(
                "legacy dylib closure exceeds bound".into(),
            ));
        }
        let mut rpaths = inherited;
        let mut is_rpath = false;
        for line in tool(&["-l"], &file)?.lines() {
            let line = line.trim();
            if line.starts_with("cmd ") {
                is_rpath = line == "cmd LC_RPATH";
            }
            if is_rpath
                && let Some(value) = line
                    .strip_prefix("path ")
                    .and_then(|line| line.split(" (offset").next())
            {
                rpaths.push(expand(value, &file));
            }
        }
        for line in tool(&["-L"], &file)?
            .lines()
            .filter(|line| line.starts_with('\t'))
        {
            let dependency = line.trim().split(" (compatibility").next().unwrap_or("");
            if dependency.starts_with("/usr/lib/") || dependency.starts_with("/System/Library/") {
                continue;
            }
            let candidates = if let Some(relative) = dependency.strip_prefix("@rpath/") {
                rpaths.iter().map(|path| path.join(relative)).collect()
            } else {
                vec![expand(dependency, &file)]
            };
            let source = candidates
                .into_iter()
                .find(|path| path.is_file())
                .ok_or_else(|| {
                    Error::Execution(format!("cannot resolve legacy linked dylib {dependency}"))
                })?;
            let source = fs::canonicalize(source)?;
            if source == file {
                continue;
            }
            if source
                .components()
                .any(|part| part.as_os_str().to_string_lossy().ends_with(".framework"))
            {
                return Err(Error::Execution(
                    "legacy framework dependency requires an explicit schema-1 runtime".into(),
                ));
            }
            let name = source
                .file_name()
                .ok_or_else(|| Error::Execution("dylib filename".into()))?
                .to_owned();
            if let Some(previous) = names.insert(name.clone(), source.clone())
                && previous != source
            {
                return Err(Error::Execution("legacy dylib filename collision".into()));
            }
            queue.push((source, rpaths.clone()));
        }
    }
    let mut output = Vec::new();
    for (name, source) in names {
        let path = stage.join(name);
        files::copy(&source, &path)?;
        output.push(path);
    }
    Ok(output)
}
pub(crate) fn linux_payload(
    payload: &Path,
    result: &Path,
    identifier: &str,
    resource_name: &str,
) -> Result<PathBuf, Error> {
    let application = result.join("application");
    files::copy(&payload.join("usr"), &application)?;
    let assets = application.join("lib").join(resource_name).join("assets");
    if assets.is_dir() {
        files::copy(
            &assets,
            &application.join("share").join(identifier).join("assets"),
        )?;
    }
    Ok(application)
}
pub(crate) fn write(
    plan: &Plan,
    package: &str,
    binary: &str,
    result: &Path,
    artifacts: &[Artifact],
    release: bool,
    target: &str,
) -> Result<(), Error> {
    let archive = artifacts
        .iter()
        .find(|artifact| matches!(artifact.format, crate::Format::Zip | crate::Format::Tar))
        .ok_or_else(|| Error::Execution("compatibility output requires ZIP or tar".into()))?;
    let artifact = if plan.platform() == Platform::Macos {
        PathBuf::from("application").join(format!(
            "{}.app",
            plan.config().name.as_deref().unwrap_or(plan.name())
        ))
    } else {
        PathBuf::from("application/bin").join(binary)
    };
    if !result.join(&artifact).exists() {
        return Err(Error::Execution("compatibility payload is missing".into()));
    }
    let mut inventory = Vec::new();
    fn visit(path: &Path, root: &Path, output: &mut Vec<serde_json::Value>) -> Result<(), Error> {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                visit(&path, root, output)?;
            } else if entry.file_type()?.is_file() {
                output.push(serde_json::json!({"path":path.strip_prefix(root).map_err(|_|Error::Execution("inventory root".into()))?,"bytes":fs::metadata(&path)?.len(),"sha256":files::digest(&path)?}));
            }
        }
        Ok(())
    }
    visit(&result.join("application"), result, &mut inventory)?;
    inventory.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    let signing = match archive.trust.signature.as_deref() {
        Some("ad_hoc") => "ad_hoc",
        Some("developer_id") => "identity",
        _ => "not_applicable",
    };
    let architecture = if target == "universal-apple-darwin" {
        "x86_64+arm64"
    } else if target.starts_with("aarch64-") {
        "arm64"
    } else {
        target.split('-').next().unwrap_or(target)
    };
    files::write_json(
        &result.join("bundle.json"),
        &serde_json::json!({"protocol":1,"package":package,"application_id":plan.identifier(),"name":plan.config().name.as_deref().unwrap_or(plan.name()),"version":plan.version(),"target":"desktop","platform":if plan.platform()==Platform::Macos{"darwin"}else{"linux"},"architecture":architecture,"profile":if release{"release"}else{"debug"},"signing":signing,"notarized":archive.trust.notarized,"artifact":artifact,"deployment":null,"archive":{"path":archive.path,"bytes":fs::metadata(result.join(&archive.path))?.len(),"sha256":archive.sha256},"files":inventory}),
    )?;
    files::write_json(
        &result.join(".rake-build.json"),
        &serde_json::json!({"package":package,"target":"desktop","command":"bundle"}),
    )?;
    fs::write(
        result.join("SHA256SUMS"),
        format!("{}  {}\n", archive.sha256, archive.path.display()),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_ownership_requires_the_exact_package_and_bundle_command() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join(".rake-build.json");
        for value in [
            serde_json::json!({"package":"app","target":"web","command":"bundle"}),
            serde_json::json!({"package":"app","target":"desktop"}),
            serde_json::json!({"package":"other","target":"desktop","command":"bundle"}),
        ] {
            files::write_json(&marker, &value).unwrap();
            assert!(!owns(root.path(), "app"));
        }
        files::write_json(
            &marker,
            &serde_json::json!({"package":"app","target":"desktop","command":"bundle"}),
        )
        .unwrap();
        assert!(owns(root.path(), "app"));
    }
}
