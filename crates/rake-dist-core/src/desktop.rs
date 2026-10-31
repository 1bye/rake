//! Common desktop registrations, with native metadata files retained as escape hatches.
use crate::Error;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Common application category, mapped by the native metadata owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ApplicationCategory {
    /// Office and business applications.
    Business,
    /// Programming and development tools.
    DeveloperTool,
    /// Learning and teaching.
    Education,
    /// Entertainment applications.
    Entertainment,
    /// Games.
    Game,
    /// Image and design tools.
    GraphicsAndDesign,
    /// Audio creation and playback.
    Music,
    /// Photography tools.
    Photography,
    /// Productivity applications.
    Productivity,
    /// General utilities.
    Utility,
    /// Video tools.
    Video,
}

impl ApplicationCategory {
    /// Returns the native bundler's stable category spelling.
    pub fn native_name(self) -> &'static str {
        match self {
            Self::Business => "Business",
            Self::DeveloperTool => "DeveloperTool",
            Self::Education => "Education",
            Self::Entertainment => "Entertainment",
            Self::Game => "Game",
            Self::GraphicsAndDesign => "GraphicsAndDesign",
            Self::Music => "Music",
            Self::Photography => "Photography",
            Self::Productivity => "Productivity",
            Self::Utility => "Utility",
            Self::Video => "Video",
        }
    }

    /// Returns the freedesktop main category list.
    pub fn linux_categories(self) -> &'static str {
        match self {
            Self::Business | Self::Productivity => "Office;",
            Self::DeveloperTool => "Development;",
            Self::Education => "Education;",
            Self::Entertainment | Self::Music => "AudioVideo;Audio;",
            Self::Game => "Game;",
            Self::GraphicsAndDesign | Self::Photography => "Graphics;",
            Self::Utility => "Utility;",
            Self::Video => "AudioVideo;Video;",
        }
    }
}

/// One native file type registration, without a leading dot in its extensions.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct FileAssociation {
    /// File suffixes, for example `rake` or `notes.json`.
    pub extensions: Vec<String>,
    /// Human type name.
    #[serde(default)]
    pub name: Option<String>,
    /// Human type description.
    #[serde(default)]
    pub description: Option<String>,
    /// Explicit MIME type for Linux registration and macOS exported metadata.
    #[serde(default)]
    pub mime_type: Option<String>,
}

/// Localized application labels. Locale keys use ASCII BCP 47 spellings.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct LocalizedMetadata {
    /// Localized display name.
    pub name: Option<String>,
    /// Localized application description.
    pub description: Option<String>,
}

/// Common OS registrations and labels, independent of any packaging engine.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct DesktopMetadata {
    /// Native category.
    pub category: Option<ApplicationCategory>,
    /// URL protocols handled by the app, without `://`.
    pub url_schemes: Vec<String>,
    /// Native file type registrations.
    pub file_associations: Vec<FileAssociation>,
    /// Labels keyed by locale, such as `fr` or `pt-PT`.
    pub localizations: BTreeMap<String, LocalizedMetadata>,
}

impl DesktopMetadata {
    /// Returns whether no common OS metadata was supplied.
    pub fn is_empty(&self) -> bool {
        self.category.is_none()
            && self.url_schemes.is_empty()
            && self.file_associations.is_empty()
            && self.localizations.is_empty()
    }

    /// Validates text, URI schemes, extensions, MIME names and safe locale identifiers.
    /// # Errors
    /// Rejects malformed registrations before invoking a native tool.
    pub fn validate(&self) -> Result<(), Error> {
        let mut schemes = std::collections::BTreeSet::new();

        for scheme in &self.url_schemes {
            if scheme.len() > 64
                || !schemes.insert(scheme.to_ascii_lowercase())
                || !scheme
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphabetic)
                || !scheme
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"+.-".contains(&b))
            {
                return Err(Error::Field("desktop.url_schemes".into()));
            }
        }

        let mut extensions = std::collections::BTreeSet::new();

        for association in &self.file_associations {
            if association.extensions.is_empty() {
                return Err(Error::Field("desktop.file_associations.extensions".into()));
            }

            for extension in &association.extensions {
                if extension.is_empty()
                    || extension.starts_with('.')
                    || extension.ends_with('.')
                    || !extension
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                    || !extensions.insert(extension.to_ascii_lowercase())
                {
                    return Err(Error::Field(
                        "desktop.file_associations.extensions must be unique portable suffixes"
                            .into(),
                    ));
                }
            }

            for text in [&association.name, &association.description]
                .into_iter()
                .flatten()
            {
                validate_text(text)?;
            }

            if let Some(mime) = &association.mime_type {
                let parts: Vec<_> = mime.split('/').collect();

                if parts.len() != 2
                    || parts.iter().any(|part| {
                        part.is_empty()
                            || !part
                                .bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b"!#$&^_.+-".contains(&b))
                    })
                {
                    return Err(Error::Field("desktop.file_associations.mime_type".into()));
                }
            }
        }

        let mut locales = std::collections::BTreeSet::new();

        for (locale, labels) in &self.localizations {
            if !locales.insert(locale.to_ascii_lowercase())
                || locale.is_empty()
                || locale.len() > 64
                || !locale
                    .split('-')
                    .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric()))
            {
                return Err(Error::Field("desktop.localizations locale".into()));
            }

            for text in [&labels.name, &labels.description].into_iter().flatten() {
                validate_text(text)?;
            }
        }

        Ok(())
    }
}

fn validate_text(value: &str) -> Result<(), Error> {
    if value.trim().is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(Error::Field(
            "desktop metadata text must be nonempty and contain no control characters".into(),
        ));
    }

    Ok(())
}
