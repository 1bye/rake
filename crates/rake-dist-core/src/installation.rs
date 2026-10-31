//! Installed public settings travel with the signed payload, not the environment.
use crate::{Format, Platform, UpdateOwner, Updates};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

/// Immutable public installation identity written before native signing.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Installation {
    /// Descriptor revision; readers reject unknown versions.
    pub schema_version: u32,
    /// Stable application identity.
    pub identifier: String,
    /// Cargo semantic version.
    pub version: String,
    /// Native version used by this installation channel (empty in older descriptors).
    #[serde(default)]
    pub package_version: String,
    /// Rust build target.
    pub target: String,
    /// Operating system.
    pub platform: Platform,
    /// Actual installation format.
    pub format: Format,
    /// Provider selected by installation channel.
    pub update_owner: UpdateOwner,
    /// Public update settings; no private credentials.
    pub updates: Updates,
    /// Helpers relative to the descriptor/resource directory.
    pub helpers: BTreeMap<String, PathBuf>,
    /// Runtime directories relative to the descriptor/resource directory.
    pub runtimes: BTreeMap<String, PathBuf>,
    /// macOS frameworks relative to Contents/Frameworks.
    #[serde(default)]
    pub frameworks: BTreeMap<String, PathBuf>,
    /// Native loader isolation groups, relative to Contents on macOS.
    #[serde(default)]
    pub native_groups: BTreeMap<String, Vec<PathBuf>>,
}

impl Installation {
    /// Canonicalizes native-generated path separators and rejects escaping entries.
    /// # Errors
    /// Rejects absolute paths, traversal and nonportable path values.
    pub fn normalize_paths(&mut self) -> Result<(), crate::Error> {
        for path in self
            .helpers
            .values_mut()
            .chain(self.runtimes.values_mut())
            .chain(self.frameworks.values_mut())
            .chain(self.updates.public_keys.values_mut())
            .chain(
                self.native_groups
                    .values_mut()
                    .flat_map(|members| members.iter_mut()),
            )
        {
            *path = crate::portable_relative_path(path)?;
        }

        Ok(())
    }
}
