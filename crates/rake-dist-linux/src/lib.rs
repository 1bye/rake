//! Linux integration, native dependency policy and dedicated Arch/Flatpak builders.
mod arch;
mod deb;
pub use deb::build_deb;
mod elf;
mod flatpak;
mod metadata;
mod rpm;
pub use arch::{ArchPackage, build_arch};
pub use elf::{ElfReport, normalize_elf};
pub use flatpak::{FlatpakBuild, FlatpakTool, build_flatpak, flatpak_manifest};
pub use metadata::{appstream, desktop_entry, validate_appstream};
pub use rpm::build_rpm;

use std::{path::PathBuf, process::Command};

/// Linux package/build failures with native diagnostic output.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Filesystem or native process failure.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Invalid integration or dependency metadata.
    #[error("Linux distribution: {0}")]
    Metadata(String),
    /// A required native tool failed.
    #[error("{program} failed: {message}")]
    Tool {
        /// Native tool name.
        program: String,
        /// Its diagnostic output.
        message: String,
    },
    /// Invalid JSON build metadata.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub(crate) fn run(command: &mut Command) -> Result<String, Error> {
    let output = command.output()?;
    if !output.status.success() {
        return Err(Error::Tool {
            program: command.get_program().to_string_lossy().into_owned(),
            message: format!(
                "{} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Native tool/runtime identity captured during a build, without credentials.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ToolRecord {
    /// Tool or SDK identity.
    pub name: String,
    /// Exact version/commit or native tool version output.
    pub version: String,
    /// Pinned archive SHA-256, when applicable.
    pub sha256: Option<String>,
}

pub(crate) fn files(root: &std::path::Path) -> Result<Vec<PathBuf>, Error> {
    let mut output = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            output.extend(files(&entry.path())?);
        } else if entry.file_type()?.is_file() {
            output.push(entry.path());
        }
    }
    output.sort();
    Ok(output)
}

pub(crate) fn shell(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
