//! Web build inputs and Wasm generation shared by native builds, bundles and dev sessions.

use super::{Args, Processes, Project, Result, string};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub(super) fn bundle_assets(
    package: &Value,
    directory: &Path,
    native: bool,
) -> Result<Option<PathBuf>> {
    let empty = serde_json::json!({});
    let config = package["metadata"]
        .get("rake")
        .and_then(|rake| rake.get("bundle"))
        .unwrap_or(&empty);
    let config = config
        .as_object()
        .ok_or("unknown or invalid package.metadata.rake.bundle configuration")?;
    let fields = [
        "name",
        "identifier",
        "assets",
        "icon",
        "build_number",
        "minimum_system_version",
        "web",
        "schema_version",
        "package_version",
        "target_overrides",
        "format_overrides",
        "desktop",
        "macos",
        "windows",
        "linux",
        "dependencies",
        "components",
        "resources",
        "sidecars",
        "runtimes",
        "updates",
        "release",
        "custom_formats",
        "component_recipes",
    ];
    if !native && config.keys().any(|key| !fields.contains(&key.as_str())) {
        return Err("unknown or invalid package.metadata.rake.bundle configuration".into());
    }
    let name = config.get("name").unwrap_or(&package["name"]);
    if name.as_str().is_none_or(|name| {
        name.trim().is_empty()
            || matches!(name, "." | "..")
            || name
                .chars()
                .any(|character| matches!(character, '/' | '\\') || (character as u32) < 32)
    }) {
        return Err(
            "bundle name must be a nonempty filename without separators or control characters"
                .into(),
        );
    }
    let default_identifier = Value::String(format!(
        "dev.rake.{}",
        string(package, "name")?.replace('_', "-")
    ));
    let identifier = config.get("identifier").unwrap_or(&default_identifier);
    if identifier.as_str().is_none_or(|identifier| {
        identifier.split('.').count() < 2
            || identifier.split('.').any(|part| {
                part.is_empty()
                    || !part
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
    }) {
        return Err(
            "bundle identifier must contain dot-separated ASCII letters, digits or hyphens".into(),
        );
    }
    let numeric = regex::Regex::new(r"^[0-9]+(?:\.[0-9]+){0,2}$")?;
    for key in ["build_number", "minimum_system_version"] {
        if let Some(value) = config.get(key).filter(|value| !value.is_null())
            && value.as_str().is_none_or(|value| !numeric.is_match(value))
        {
            return Err(
                "build_number/minimum_system_version must contain one to three numeric components"
                    .into(),
            );
        }
    }
    let assets = input(config.get("assets"), directory, true)?;
    input(config.get("icon"), directory, false)?;
    let web = config.get("web").unwrap_or(&empty);
    let web = web.as_object().ok_or("unknown bundle.web configuration")?;
    if web
        .keys()
        .any(|key| !["base_path", "routing", "offline"].contains(&key.as_str()))
    {
        return Err("unknown bundle.web configuration".into());
    }
    valid_base(web.get("base_path").unwrap_or(&Value::String("/".into())))?;
    if web.get("offline").is_some_and(|value| !value.is_boolean()) {
        return Err("bundle.web.offline must be a boolean".into());
    }
    if web
        .get("routing")
        .is_some_and(|value| value != "path" && value != "hash")
    {
        return Err("bundle.web.routing must be path or hash".into());
    }

    Ok(assets)
}

pub(super) fn valid_base(value: &Value) -> Result<()> {
    if value.as_str().is_none_or(|value| {
        !value.starts_with('/')
            || !value.ends_with('/')
            || (value != "/"
                && value[1..value.len() - 1].split('/').any(|part| {
                    part.is_empty()
                        || matches!(part, "." | "..")
                        || !part
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || b"._~-".contains(&byte))
                }))
    }) {
        return Err("Web base_path must be / or an ASCII path with leading/trailing slashes and no dot segments".into());
    }

    Ok(())
}

fn input(value: Option<&Value>, directory: &Path, is_directory: bool) -> Result<Option<PathBuf>> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let value = value
        .as_str()
        .filter(|value| !value.is_empty() && !Path::new(value).is_absolute())
        .ok_or("bundle inputs must be relative to the package directory")?;
    let path = directory.join(value);
    if path.is_symlink() {
        return Err(format!("bundle inputs cannot be symlinks: {value}").into());
    }
    let path = path.canonicalize()?;
    let directory = directory.canonicalize()?;
    if path == directory
        || !path.starts_with(directory)
        || if is_directory {
            !path.is_dir()
        } else {
            !path.is_file()
        }
    {
        return Err(format!("invalid bundle input: {value}").into());
    }

    Ok(Some(path))
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    super::permissions::set(destination, 0o755)?;
    let mut entries = fs::read_dir(source)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let source = entry.path();
        let target = destination.join(entry.file_name());
        let kind = fs::symlink_metadata(&source)?.file_type();
        if !kind.is_dir() && !kind.is_file() {
            return Err(format!(
                "asset input must be a regular file/directory, not a link or special file: {}",
                source.display()
            )
            .into());
        }
        if target.exists() {
            return Err(format!(
                "asset/output collision: {}",
                target.file_name().unwrap_or_default().to_string_lossy()
            )
            .into());
        }
        if kind.is_dir() {
            copy_tree(&source, &target)?;
        } else {
            fs::copy(&source, &target)?;
            super::permissions::set(&target, 0o644)?;
        }
    }

    Ok(())
}

pub(super) fn build(
    project: &Project,
    args: &Args,
    processes: &Processes<'_>,
    output: &Path,
) -> Result<PathBuf> {
    fs::create_dir_all(output)?;
    let mut command = Command::new("cargo");
    command
        .args([
            "build",
            "--locked",
            "--message-format=json-render-diagnostics",
            "--manifest-path",
        ])
        .arg(string(&project.package, "manifest_path")?)
        .arg("--package")
        .arg(string(&project.package, "name")?)
        .current_dir(&project.root);
    if let Some(features) = &args.features {
        command.arg("--features").arg(features);
    }
    if args.release {
        command.arg("--release");
    }
    command.args(["--lib", "--target", "wasm32-unknown-unknown"]);
    let mut artifacts = Vec::new();
    processes.run_output(&mut command, |message| {
        if message["package_id"] == project.package["id"]
            && message["target"]["name"] == project.target
        {
            artifacts.push(message);
        }
    })?;
    if artifacts.is_empty() {
        return Err("cargo did not report an application artifact".into());
    }
    let source = artifacts
        .iter()
        .rev()
        .filter_map(|artifact| artifact["filenames"].as_array())
        .flatten()
        .filter_map(Value::as_str)
        .find(|name| name.ends_with(".wasm"))
        .ok_or("cargo did not report a Wasm artifact")?;
    if project.web_dir.is_dir() {
        copy_tree(&project.web_dir, output)?;
    }
    if let Some(assets) = &project.assets {
        copy_tree(assets, &output.join("assets"))?;
    }
    fs::write(
        output.join("rake-web-font-licenses.txt"),
        include_bytes!("../../scripts/rake-web-font-licenses.txt"),
    )?;
    if output.join("pkg").exists() {
        return Err("web/pkg is reserved for generated Wasm bindings".into());
    }
    let mut command = Command::new(project.bindgen.as_ref().ok_or("wasm-bindgen missing")?);
    command
        .args(["--target", "web", "--out-dir"])
        .arg(output.join("pkg"))
        .arg(source);
    processes.run_output(&mut command, |_| {})?;
    if !output.join("index.html").is_file() {
        fs::write(
            output.join("index.html"),
            format!(
                "<!doctype html><html><head><meta charset='utf-8'><title>Rake</title>\
<style>html,body,canvas{{margin:0;width:100%;height:100%;display:block}}</style></head><body>\
<script type='module'>import init from '/pkg/{}.js';await init();</script></body></html>",
                project.target
            ),
        )?;
    }

    super::favicon::install(output)?;

    Ok(output.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn web_metadata_rejects_nonboolean_offline_and_accepts_native_sections() {
        let root = tempfile::tempdir().unwrap();
        let mut package = json!({"name":"example","version":"1.0.0","metadata":{"rake":{"bundle":{
            "schema_version":1,"web":{"offline":true},"windows":{"manifest":"windows.manifest.xml"},
            "macos":{"formats":["dmg"]},"linux":{"formats":["deb"]},"resources":[],"updates":{"enabled":false}
        }}}});
        assert!(
            bundle_assets(&package, root.path(), false)
                .unwrap()
                .is_none()
        );
        package["metadata"]["rake"]["bundle"]["web"]["offline"] = json!("yes");
        assert!(
            bundle_assets(&package, root.path(), false)
                .unwrap_err()
                .to_string()
                .contains("boolean")
        );
    }

    #[test]
    fn explicit_bundle_inputs_preserve_original_validation_cases() {
        let root = tempfile::tempdir().unwrap();
        assert!(
            bundle_assets(
                &json!({"name":"fixture","version":"1.0.0","metadata":null}),
                root.path(),
                false
            )
            .unwrap()
            .is_none()
        );
        for invalid in [
            "", "notes/", "/notes", "//notes/", "/a/../", "/a/./", "/a//b/", "/a%2fb/", "/a?b/",
            "/漢/",
        ] {
            assert!(valid_base(&json!(invalid)).is_err(), "{invalid}");
        }
        for config in [
            json!({"identifier":"not an id"}),
            json!({"name":"../escape"}),
            json!({"assets":"../"}),
            json!({"typo":true}),
            json!({"web":{"routing":"wrong"}}),
        ] {
            assert!(bundle_assets(&json!({"name":"fixture","version":"1.0.0","metadata":{"rake":{"bundle":config}}}),root.path(),false).is_err());
        }
        let source = root.path().join("assets");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("file"), "one").unwrap();
        copy_tree(&source, &root.path().join("copy")).unwrap();
        assert!(copy_tree(&source, &root.path().join("copy")).is_err());
        #[cfg(unix)]
        {
            use std::{ffi::CString, os::unix::fs::symlink};
            symlink("/etc/passwd", source.join("link")).unwrap();
            assert!(copy_tree(&source, &root.path().join("copy2")).is_err());
            fs::remove_file(source.join("link")).unwrap();
            let path = CString::new(source.join("pipe").as_os_str().as_encoded_bytes()).unwrap();
            // SAFETY: path is a live NUL-terminated fixture path; create only this FIFO.
            assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
            assert!(copy_tree(&source, &root.path().join("copy3")).is_err());
        }
    }
}
