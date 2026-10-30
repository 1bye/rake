//! Typed partial configuration used by target and installation-format layers.
use crate::{Component, Config, Format, Platform, Resource, Runtime, Sidecar};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

/// Partial settings applied after platform selection; arrays replace prior arrays.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct Overrides {
    /// Display name override.
    pub name: Option<String>,
    /// Icon override.
    pub icon: Option<PathBuf>,
    /// Asset tree override.
    pub assets: Option<PathBuf>,
    /// Explicit native numeric version where SemVer cannot be represented.
    pub package_version: Option<String>,
    /// Bundle build counter.
    pub build_number: Option<String>,
    /// Replacement list of output formats; only valid at the target layer.
    pub formats: Option<Vec<Format>>,
    /// Replacement resource mappings.
    pub resources: Option<Vec<Resource>>,
    /// Replacement component source list.
    pub components: Option<Vec<Component>>,
    /// Replacement helper list.
    pub sidecars: Option<Vec<Sidecar>>,
    /// Replacement runtime list.
    pub runtimes: Option<Vec<Runtime>>,
    /// Partial update settings.
    pub updates: Option<UpdateOverrides>,
}

/// Partial public update configuration; unspecified fields retain their values.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct UpdateOverrides {
    /// Explicitly enable or disable update support.
    pub enabled: Option<bool>,
    /// Release channel override.
    pub channel: Option<String>,
    /// Public feed location override.
    pub base_url: Option<String>,
    /// Public key entries merged by provider name.
    pub public_keys: BTreeMap<String, PathBuf>,
}

impl Overrides {
    pub(crate) fn apply(&self, config: &mut Config, platform: Platform) {
        macro_rules! replace {
            ($($field:ident),*) => { $(if let Some(value) = &self.$field {config.$field = Some(value.clone());})* };
        }
        replace!(name, icon, assets, package_version, build_number);
        if let Some(value) = &self.resources {
            config.resources.clone_from(value);
        }
        if let Some(value) = &self.components {
            config.components.clone_from(value);
        }
        if let Some(value) = &self.sidecars {
            config.sidecars.clone_from(value);
        }
        if let Some(value) = &self.runtimes {
            config.runtimes.clone_from(value);
        }
        if let Some(value) = &self.formats {
            *config.formats_mut(platform) = value.clone();
        }
        if let Some(value) = &self.updates {
            if let Some(enabled) = value.enabled {
                config.updates.enabled = enabled;
            }
            if let Some(channel) = &value.channel {
                config.updates.channel = Some(channel.clone());
            }
            if let Some(base) = &value.base_url {
                config.updates.base_url = Some(base.clone());
            }
            config.updates.public_keys.extend(value.public_keys.clone());
        }
    }
}
