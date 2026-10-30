//! Distribution planning at the Cargo package boundary.
//!
//! Native execution is separate from the portable plan returned here.

pub use rake_dist_core::{Config, Plan, Platform};
use serde::Deserialize;
use std::{path::Path, process::Command};

/// Errors loading the selected package and its distribution configuration.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Cargo could not be started.
    #[error("cannot run cargo metadata")]
    Io(#[from] std::io::Error),
    /// Cargo rejected the manifest or locked dependency resolution.
    #[error("cargo metadata failed: {0}")]
    Cargo(String),
    /// Cargo/config JSON is malformed.
    #[error("invalid Cargo or distribution metadata")]
    Json(#[from] serde_json::Error),
    /// Workspace package selection is ambiguous or absent.
    #[error("select one workspace package with --package")]
    Package,
    /// Portable validation rejected the configuration.
    #[error(transparent)]
    Config(#[from] rake_dist_core::Error),
}

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    workspace_members: Vec<String>,
}

#[derive(Deserialize)]
struct Package {
    id: String,
    name: String,
    version: String,
    metadata: serde_json::Value,
}

/// Loads a locked/offline Cargo package and validates its portable plan.
///
/// No builds, downloads, signing or publication are performed. Cargo must be
/// available and dependency resolution must already be locked for this project.
///
/// # Errors
/// Returns errors for Cargo execution, ambiguous package selection, malformed
/// metadata or an invalid distribution configuration.
pub fn plan(manifest: &Path, package: Option<&str>, platform: Platform) -> Result<Plan, Error> {
    let output = Command::new("cargo")
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--locked",
            "--offline",
            "--manifest-path",
        ])
        .arg(manifest)
        .output()?;
    if !output.status.success() {
        return Err(Error::Cargo(
            String::from_utf8_lossy(&output.stderr).trim().into(),
        ));
    }
    plan_metadata(&output.stdout, package, platform)
}

/// Resolves an existing Cargo metadata response without running any tools.
///
/// # Errors
/// Returns errors for malformed metadata, ambiguous package selection or invalid
/// distribution settings. Only workspace members may be selected.
pub fn plan_metadata(
    metadata: &[u8],
    package: Option<&str>,
    platform: Platform,
) -> Result<Plan, Error> {
    let metadata: Metadata = serde_json::from_slice(metadata)?;
    let mut candidates = metadata.packages.into_iter().filter(|item| {
        metadata.workspace_members.contains(&item.id)
            && package.is_none_or(|name| item.name == name)
    });
    let selected = candidates.next().ok_or(Error::Package)?;
    if candidates.next().is_some() {
        return Err(Error::Package);
    }
    let config = selected.metadata.pointer("/rake/bundle");
    let config: Config = match config {
        Some(value) => serde_json::from_value(value.clone())?,
        None => Config::default(),
    };
    Ok(config.plan(&selected.name, &selected.version, platform)?)
}

/// Returns the versioned configuration schema for editors and external callers.
pub fn configuration_schema() -> serde_json::Value {
    schemars::schema_for!(Config).to_value()
}
