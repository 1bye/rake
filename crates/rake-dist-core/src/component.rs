//! Pinned native component inputs, independent of packaging tools.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Archive encoding of a pinned HTTPS component.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Archive {
    /// One uncompressed payload file.
    #[default]
    File,
    /// ZIP archive.
    Zip,
    /// Gzip-compressed tar archive.
    TarGz,
    /// XZ-compressed tar archive.
    TarXz,
}

/// Explicit local directory or authenticated HTTPS/hash-pinned component.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComponentSource {
    /// Package-relative, checked-in or developer-prepared component.
    Local {
        /// Package-relative directory/file.
        path: PathBuf,
    },
    /// Downloaded bytes are accepted only when their SHA-256 matches.
    Https {
        /// Public HTTPS URL without credentials.
        url: String,
        /// Lowercase 64-digit SHA-256.
        sha256: String,
        /// How to unpack the payload.
        #[serde(default)]
        archive: Archive,
        /// Upper bound on compressed bytes; default 512 MiB.
        #[serde(default = "download_limit")]
        max_download_bytes: u64,
    },
}

fn download_limit() -> u64 {
    512 * 1024 * 1024
}

/// One named, exact-target native component. Several targets may share a name.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Component {
    /// Name referenced by a helper or runtime recipe.
    pub name: String,
    /// Exact Rust target triple; never inferred from the developer's machine.
    pub target: String,
    /// Source with explicit integrity policy.
    pub source: ComponentSource,
    /// License/notice files relative to the prepared component root.
    #[serde(default)]
    pub licenses: Vec<PathBuf>,
}
