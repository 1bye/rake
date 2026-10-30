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
}
