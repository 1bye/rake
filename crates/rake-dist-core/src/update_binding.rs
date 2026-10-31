//! Update authority embedded by the application's build script.
use crate::{Format, Platform, UpdateOwner};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Immutable application-side policy; installed metadata cannot grant extra authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateBinding {
    /// Policy revision.
    pub schema_version: u32,
    /// Stable application identity.
    pub identifier: String,
    /// Cargo application version.
    pub version: String,
    /// Native package/update version.
    pub package_version: String,
    /// Exact compiled Rust target.
    pub target: String,
    /// Compiled native packaging format.
    pub format: Format,
    /// Update authority selected for that packaging format.
    pub owner: UpdateOwner,
    /// Bound update channel.
    pub channel: String,
    /// Bound HTTPS publication root.
    pub base_url: Option<String>,
    /// Public verification key contents embedded into the executable.
    pub public_keys: BTreeMap<String, Vec<u8>>,
}

impl UpdateBinding {
    /// Checks serialized policy before provider activation.
    /// # Errors
    /// Rejects unknown revisions, invalid versions/targets, channel traversal,
    /// conflicting format authority and non-HTTPS publication roots.
    pub fn validate(&self) -> Result<(), crate::Error> {
        if self.schema_version != 1 {
            return Err(crate::Error::Field("update binding schema".into()));
        }

        semver::Version::parse(&self.version)
            .map_err(|_| crate::Error::Field("update binding version".into()))?;

        if matches!(self.format, Format::Appimage | Format::Velopack) {
            semver::Version::parse(&self.package_version)
                .map_err(|_| crate::Error::Field("native updater version".into()))?;
        }

        let platform = if self.target == "universal-apple-darwin" {
            Platform::Macos
        } else {
            Platform::from_target(&self.target)?
        };

        if !self.format.supports(platform)
            || (self.owner != UpdateOwner::Disabled
                && self.owner != self.format.update_owner(platform))
        {
            return Err(crate::Error::Field(
                "update binding owner conflicts with compiled format".into(),
            ));
        }

        if self.identifier.is_empty()
            || self.channel.is_empty()
            || !self
                .channel
                .bytes()
                .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'-' | b'_' | b'.'))
            || self.channel == "."
            || self.channel == ".."
        {
            return Err(crate::Error::Field("update identity/channel".into()));
        }

        if self.owner != UpdateOwner::Disabled {
            let url = self
                .base_url
                .as_deref()
                .and_then(|value| url::Url::parse(value).ok())
                .ok_or_else(|| crate::Error::Field("update binding base URL".into()))?;

            if url.scheme() != "https"
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(crate::Error::Field(
                    "update binding requires public HTTPS".into(),
                ));
            }
        }

        Ok(())
    }
}

impl Format {
    /// Returns the native update authority for this format, before updates are disabled.
    pub fn update_owner(self, platform: Platform) -> UpdateOwner {
        match self {
            Self::App | Self::Dmg | Self::Zip if platform == Platform::Macos => {
                UpdateOwner::Sparkle
            }
            Self::Msix => UpdateOwner::AppInstaller,
            Self::Velopack => UpdateOwner::Velopack,
            Self::Deb => UpdateOwner::Apt,
            Self::Rpm => UpdateOwner::RpmRepository,
            Self::Arch => UpdateOwner::Pacman,
            Self::Flatpak => UpdateOwner::Flatpak,
            Self::Appimage => UpdateOwner::Appimage,
            _ => UpdateOwner::External,
        }
    }
}
