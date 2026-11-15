//! Diagnose unoptimized GPUI dependencies using the workspace's development profile.

use super::{Args, Capture, Processes, Result};
use serde_json::Value;
use std::{fs, path::Path, process::Command};

const SNIPPET: &str = "[profile.dev]\nopt-level = 1\n\n[profile.dev.package.\"*\"]\nopt-level = 3";

pub(super) fn warn(args: &Args, processes: &Processes<'_>) -> Result<()> {
    let mut command = Command::new("cargo");

    command
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(&args.manifest);
    let metadata: Value =
        serde_json::from_str(&processes.capture(&mut command, Capture::Metadata)?)?;

    warn_metadata(&metadata, processes)
}

pub(super) fn warn_metadata(metadata: &Value, processes: &Processes<'_>) -> Result<()> {
    let root = metadata["workspace_root"]
        .as_str()
        .ok_or("missing workspace root")?;

    let environment = std::env::var("CARGO_PROFILE_DEV_OPT_LEVEL").ok();

    let members = metadata["workspace_members"]
        .as_array()
        .ok_or("missing workspace members")?;

    for package in metadata["packages"].as_array().ok_or("missing packages")? {
        let Some(name @ ("gpui" | "gpui-pre" | "gpui-fast")) = package["name"].as_str() else {
            continue;
        };

        let mut manifest: toml::Value =
            toml::from_str(&fs::read_to_string(Path::new(root).join("Cargo.toml"))?)?;

        if let Some(overrides) = manifest
            .get_mut("profile")
            .and_then(|p| p.get_mut("dev"))
            .and_then(|p| p.get_mut("package"))
            .and_then(toml::Value::as_table_mut)
        {
            for (spec, settings) in overrides.clone() {
                if spec == "*" {
                    continue;
                }

                // Let Cargo resolve version abbreviations and source-qualified package IDs.
                let mut command = Command::new("cargo");

                command
                    .args(["pkgid", "--manifest-path"])
                    .arg(Path::new(root).join("Cargo.toml"))
                    .args(["--package", &spec]);
                let id = processes.capture(&mut command, Capture::Metadata)?;

                if package["id"].as_str() == Some(id.trim()) {
                    overrides.insert(name.to_owned(), settings);
                }
            }
        }

        let version = package["version"]
            .as_str()
            .ok_or("missing package version")?;

        if unoptimized(
            &manifest,
            name,
            version,
            members.contains(&package["id"]),
            environment.as_deref(),
        ) {
            eprintln!(
                "rake: warning: {name} builds at dev opt-level 0; scrolling may stutter. Add to {root}/Cargo.toml:\n\n{SNIPPET}\n"
            );
        }
    }

    Ok(())
}

fn unoptimized(
    manifest: &toml::Value,
    name: &str,
    version: &str,
    member: bool,
    environment: Option<&str>,
) -> bool {
    let profile = manifest.get("profile").and_then(|p| p.get("dev"));

    let Some(profile) = profile else {
        return environment.unwrap_or("0") == "0";
    };

    let overrides = profile.get("package").and_then(toml::Value::as_table);

    let named = overrides.and_then(|packages| {
        packages
            .get(format!("{name}:{version}").as_str())
            .or_else(|| packages.get(format!("{name}@{version}").as_str()))
            .or_else(|| packages.get(name))
    });

    let wildcard = overrides
        .and_then(|packages| packages.get("*"))
        .filter(|_| !member);

    let level = named
        .and_then(|p| p.get("opt-level"))
        .or_else(|| wildcard.and_then(|p| p.get("opt-level")));

    if let Some(level) = level {
        return level.as_integer() == Some(0) || level.as_str() == Some("0");
    }

    if let Some(level) = environment {
        return level == "0";
    }

    profile
        .get("opt-level")
        .is_none_or(|level| level.as_integer() == Some(0) || level.as_str() == Some("0"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unrelated_packages_need_no_profile_metadata() {
        let cancelled = std::sync::atomic::AtomicBool::new(false);
        let processes = Processes::new(&cancelled);

        let metadata = serde_json::json!({
            "workspace_root": "/unused",
            "workspace_members": [],
            "packages": [{"id": "local", "source": null}]
        });

        assert!(warn_metadata(&metadata, &processes).is_ok());
    }

    #[test]
    fn effective_gpui_optimization() {
        for (text, member, env, expected) in [
            ("", false, None, true),
            ("", false, Some("3"), false),
            ("[profile.dev]\nopt-level=1", false, Some("0"), true),
            (
                "[profile.dev.package.'*']\nopt-level=3",
                false,
                Some("0"),
                false,
            ),
            ("[profile.dev.package.'*']\nopt-level=3", true, None, true),
            (
                "[profile.dev.package.'*']\nopt-level=3\n[profile.dev.package.gpui-pre]\nopt-level=0",
                false,
                None,
                true,
            ),
            (
                "[profile.dev.package.'gpui-pre@0.3.11']\nopt-level=3",
                false,
                Some("0"),
                false,
            ),
            ("[profile.dev]\nopt-level='z'", false, None, false),
        ] {
            let manifest = toml::from_str(text).unwrap();

            assert_eq!(
                unoptimized(&manifest, "gpui-pre", "0.3.11", member, env),
                expected,
                "{text}, {env:?}"
            );
        }
    }
}
