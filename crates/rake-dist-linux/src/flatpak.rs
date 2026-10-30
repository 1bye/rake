//! Flatpak builds from vendored source inside the selected SDK.
use crate::{Error, ToolRecord, run, shell};
use rake_dist_core::FlatpakConfig;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Verified Rust component archive installed only inside the Flatpak build sandbox.
#[derive(Clone, Debug)]
pub struct FlatpakTool {
    /// Rust distribution component, such as rustc or cargo.
    pub name: String,
    /// Local verified archive.
    pub archive: PathBuf,
    /// Pinned SHA-256.
    pub sha256: String,
}

/// One source build, with immutable SDK metadata recorded alongside its artifact.
#[derive(Debug)]
pub struct FlatpakBuild<'a> {
    /// Validated reverse-DNS application identity.
    pub identifier: &'a str,
    /// Rust target triple.
    pub target: &'a str,
    /// Flatpak runtime, SDK, permissions and optional complete manifest.
    pub config: &'a FlatpakConfig,
    /// Generated manifest path (or copied validated override).
    pub manifest: &'a Path,
    /// Dedicated native job directory.
    pub output: &'a Path,
}

/// Generates a source-build manifest using an explicitly pinned Rust toolchain.
///
/// The source tree must contain a Cargo vendor configuration and all dependencies.
/// `payload` contains resources and desktop metadata, without the host executable.
/// # Errors
/// Rejects invalid relative paths, missing compiler components and non-UTF-8 paths.
#[allow(clippy::too_many_arguments)]
pub fn flatpak_manifest(
    identifier: &str,
    binary: &str,
    package: &str,
    manifest: &Path,
    source: &Path,
    payload: &Path,
    config: &FlatpakConfig,
    tools: &[FlatpakTool],
    features: &[String],
) -> Result<Value, Error> {
    rake_dist_core::validate_relative_path(manifest)
        .map_err(|error| Error::Metadata(error.to_string()))?;
    for component in ["rustc", "cargo", "rust-std"] {
        if !tools.iter().any(|tool| tool.name == component) {
            return Err(Error::Metadata(format!(
                "Flatpak Rust {component} archive missing"
            )));
        }
    }
    let mut sources = Vec::new();
    let mut commands = Vec::new();
    for tool in tools {
        sources.push(json!({"type":"archive","path":tool.archive,"sha256":tool.sha256,"archive-type":"tar-xz","dest":tool.name}));
        commands.push(format!(
            "cd {} && ./install.sh --prefix=/app/.rake-build/rust --disable-ldconfig",
            shell(&tool.name)
        ));
    }
    let feature_args = if features.is_empty() {
        String::new()
    } else {
        format!(" --features {}", shell(&features.join(",")))
    };
    let compile = format!(
        "cargo build --offline --locked --release --manifest-path {} --package {} --bin {}{feature_args}",
        shell(&manifest.to_string_lossy()),
        shell(package),
        shell(binary)
    );
    Ok(json!({
        "app-id":identifier,"runtime":config.runtime,"runtime-version":config.runtime_version,"sdk":config.sdk,
        "command":binary,"finish-args":config.finish_args,
        "build-options":{"env":{"CARGO_HOME":"/run/build/rake-application/.cargo-home","CARGO_TARGET_DIR":"/run/build/rake-application/target","RAKE_DISTRIBUTION_FORMAT":"flatpak","CARGO_PROFILE_RELEASE_DEBUG":"0","CARGO_INCREMENTAL":"0"}},
        "modules":[
            {"name":"rake-rust-toolchain","buildsystem":"simple","build-options":{"strip":false,"no-debuginfo":true},"build-commands":commands,"sources":sources,"cleanup":["*"]},
            {"name":"rake-application","buildsystem":"simple","build-options":{"prepend-path":"/app/.rake-build/rust/bin"},
             "build-commands":[compile,format!("install -Dm755 target/release/{} /app/bin/{}",shell(binary),shell(binary)),"cp -a payload/usr/. /app/"],
             "sources":[{"type":"dir","path":source},{"type":"dir","path":payload,"dest":"payload"}]}
        ],"cleanup":["/.rake-build"]
    }))
}

/// Builds a bundle through flatpak-builder and captures installed runtime/SDK commits.
/// # Errors
/// Returns identity conflicts, SDK pin mismatches, missing native prerequisites or
/// actual sandbox compilation/build failures. It never installs a remote or SDK.
pub fn build_flatpak(input: &FlatpakBuild<'_>) -> Result<(PathBuf, Vec<ToolRecord>), Error> {
    let manifest: Value = serde_json::from_slice(&fs::read(input.manifest)?)?;
    if manifest["app-id"] != input.identifier
        || manifest["runtime"] != input.config.runtime
        || manifest["sdk"] != input.config.sdk
        || manifest["runtime-version"] != input.config.runtime_version
    {
        return Err(Error::Metadata(
            "Flatpak manifest identity/runtime conflicts with configuration".into(),
        ));
    }
    let arch = match input.target.split('-').next() {
        Some("x86_64") => "x86_64",
        Some("aarch64") => "aarch64",
        _ => return Err(Error::Metadata("unsupported Flatpak architecture".into())),
    };
    fs::create_dir_all(input.output)?;
    let mut records = vec![ToolRecord {
        name: "flatpak-builder".into(),
        version: run(Command::new("flatpak-builder").arg("--version"))?
            .trim()
            .into(),
        sha256: None,
    }];
    for (name, pinned) in [
        (&input.config.runtime, &input.config.runtime_commit),
        (&input.config.sdk, &input.config.sdk_commit),
    ] {
        let reference = format!("{name}/{arch}/{}", input.config.runtime_version);
        let commit = run(Command::new("flatpak").args(["info", "--show-commit", &reference]))?
            .trim()
            .to_owned();
        if pinned.as_ref().is_some_and(|expected| *expected != commit) {
            return Err(Error::Metadata(format!(
                "installed Flatpak {reference} commit differs from configured pin"
            )));
        }
        records.push(ToolRecord {
            name: reference,
            version: commit,
            sha256: None,
        });
    }
    let repo = input.output.join("repo");
    run(Command::new("flatpak-builder")
        .args([
            "--force-clean",
            "--disable-rofiles-fuse",
            &format!("--arch={arch}"),
            &format!("--repo={}", repo.display()),
            &format!("--state-dir={}", input.output.join("state").display()),
        ])
        .arg(input.output.join("build"))
        .arg(input.manifest))?;
    let artifact = input.output.join(format!("{}.flatpak", input.identifier));
    run(Command::new("flatpak")
        .args(["build-bundle", &format!("--arch={arch}")])
        .arg(repo)
        .arg(&artifact)
        .args([input.identifier, "master"]))?;
    Ok((artifact, records))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sandbox_builds_source_and_does_not_inherit_network_permissions() {
        let tools = ["rustc", "cargo", "rust-std"].map(|name| FlatpakTool {
            name: name.into(),
            archive: PathBuf::from(format!("/tmp/{name}.tar.xz")),
            sha256: "a".repeat(64),
        });
        let manifest = flatpak_manifest(
            "dev.rake.Example",
            "example",
            "example",
            Path::new("examples/app/Cargo.toml"),
            Path::new("/tmp/source"),
            Path::new("/tmp/payload"),
            &FlatpakConfig::default(),
            &tools,
            &[],
        )
        .unwrap();
        assert_eq!(manifest["modules"][0]["build-options"]["strip"], false);
        assert_eq!(
            manifest["modules"][0]["build-options"]["no-debuginfo"],
            true
        );
        assert!(
            manifest["modules"][1]["build-commands"][0]
                .as_str()
                .unwrap()
                .contains("cargo build --offline --locked")
        );
        assert!(
            !manifest["finish-args"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == "--share=network")
        );
        assert!(
            flatpak_manifest(
                "dev.rake.Example",
                "example",
                "example",
                Path::new("../Cargo.toml"),
                Path::new("/tmp/source"),
                Path::new("/tmp/payload"),
                &FlatpakConfig::default(),
                &tools,
                &[]
            )
            .is_err()
        );
    }
}
