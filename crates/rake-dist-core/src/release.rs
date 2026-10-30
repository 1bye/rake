//! Public release destinations and local credential references.
use crate::{Error, Format};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

/// Publication policy shared by native workers and release promotion.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct ReleaseConfig {
    /// Explicit publication destination; absent means bundle-only.
    pub publisher: Option<Publisher>,
    /// Complete matrix required before any channel promotion, keyed by Rust target.
    pub required: BTreeMap<String, Vec<Format>>,
    /// Custom backend names required on each target before channel promotion.
    pub custom_required: BTreeMap<String, Vec<String>>,
    /// Named provider signing commands. Input is exact message bytes on stdin;
    /// output is a raw 64-byte Ed25519 signature. Commands keep private keys in
    /// the selected local signing profile. No shell expansion occurs.
    pub signers: BTreeMap<String, Vec<String>>,
    /// Sparkle SDK generate_appcast executable, from the pinned SDK.
    pub sparkle_tool: Option<PathBuf>,
    /// Existing Sparkle Keychain account; defaults to the SDK's default account.
    pub sparkle_account: Option<String>,
    /// Existing Linux GPG signing identity (fingerprint, never private key bytes).
    pub gpg_key: Option<String>,
    /// Native Linux repository formats to build for each exact target.
    pub repositories: BTreeMap<String, Vec<Format>>,
    /// Feed validity in seconds; default is seven days.
    pub feed_validity_seconds: Option<u64>,
}
/// Explicit destinations. Remote destinations are never inferred from app identity.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Publisher {
    /// Export to an owned directory; publish its contents through a static HTTPS server.
    Directory {
        /// Local repository root, relative to the Cargo package or absolute.
        path: PathBuf,
        /// Public HTTPS URL corresponding to this root.
        public_url: String,
    },
    /// Upload a draft release, verify every asset, then expose it as the latest release.
    Github {
        /// Explicit owner/repository.
        repository: String,
        /// Existing immutable Git tag; creation of tags is outside publication.
        tag: String,
    },
    /// HTTPS object service with conditional, transactional promotion.
    /// `PUT /objects/<sha256>-<filename>` stores immutable bytes. GET verifies them.
    /// `GET /channels/<channel>` returns a JSON state and ETag (404 means absent).
    /// PUT to that channel with If-Match/If-None-Match atomically replaces the
    /// whole native-feed tree declared by the request; a static server can serve it.
    Https {
        /// HTTPS service root, without credentials.
        url: String,
        /// Environment variable containing a scoped bearer token.
        token_env: String,
        /// Public static HTTPS root serving promoted native feed trees.
        public_url: String,
    },
}
impl ReleaseConfig {
    /// Validates public destination and matrix settings without loading credentials.
    /// # Errors
    /// Rejects ambiguous destinations, malformed targets, empty commands and unsafe URLs.
    pub fn validate(&self) -> Result<(), Error> {
        for (target, formats) in &self.required {
            let platform = if target == "universal-apple-darwin" {
                crate::Platform::Macos
            } else {
                crate::Platform::from_target(target)?
            };
            if formats.is_empty() || formats.iter().any(|format| !format.supports(platform)) {
                return Err(Error::Field("release.required formats".into()));
            }
        }
        for (target, names) in &self.custom_required {
            if target != "universal-apple-darwin" {
                crate::Platform::from_target(target)?;
            }
            if names.is_empty() {
                return Err(Error::Field(
                    "release.custom_required cannot be empty".into(),
                ));
            }
            for name in names {
                crate::component_name(name)?;
            }
        }
        for (target, formats) in &self.repositories {
            if crate::Platform::from_target(target)? != crate::Platform::Linux
                || formats.iter().any(|format| {
                    !matches!(
                        format,
                        Format::Deb | Format::Rpm | Format::Arch | Format::Flatpak
                    )
                })
                || self.gpg_key.is_none()
            {
                return Err(Error::Field(
                    "release.repositories requires Linux formats and gpg_key".into(),
                ));
            }
        }
        for command in self.signers.values() {
            if command.is_empty() || command.iter().any(|arg| arg.contains('\0')) {
                return Err(Error::Field("release.signers argv".into()));
            }
        }
        if self
            .feed_validity_seconds
            .is_some_and(|value| !(300..=90 * 86400).contains(&value))
        {
            return Err(Error::Field(
                "release.feed_validity_seconds must be 300..7776000".into(),
            ));
        }
        match &self.publisher {
            Some(Publisher::Directory { public_url, .. }) => https(public_url)?,
            Some(Publisher::Https {
                url,
                public_url,
                token_env,
            }) => {
                https(url)?;
                https(public_url)?;
                if token_env.is_empty()
                    || !token_env
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'_')
                {
                    return Err(Error::Field("release publisher token_env".into()));
                }
            }
            Some(Publisher::Github { repository, tag }) => validate_github(repository, tag)?,
            None => {}
        }
        Ok(())
    }
}
fn https(value: &str) -> Result<(), Error> {
    let url = url::Url::parse(value).map_err(|_| Error::Field("release HTTPS URL".into()))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::Field("release requires a public HTTPS URL".into()));
    }
    Ok(())
}
/// Resolves native feed URLs, including GitHub's flat release-asset namespace.
/// # Errors
/// Rejects unsafe channel/target/file path segments or non-HTTPS roots.
pub fn feed_url(base: &str, channel: &str, target: &str, file: &str) -> Result<String, Error> {
    https(base)?;
    for part in [channel, target, file] {
        if part.is_empty()
            || part == "."
            || part == ".."
            || !part
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
        {
            return Err(Error::Field("update feed path segment".into()));
        }
    }
    let root = base.trim_end_matches('/');
    if root.starts_with("https://github.com/") && root.ends_with("/releases/latest/download") {
        Ok(format!("{root}/{channel}-{target}-{file}"))
    } else {
        Ok(format!("{root}/{channel}/{target}/{file}"))
    }
}

fn validate_github(repository: &str, tag: &str) -> Result<(), Error> {
    if repository.split('/').count() != 2
        || repository.split('/').any(|part| {
            part.is_empty()
                || !part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        })
        || tag.is_empty()
        || tag.starts_with('-')
        || tag.contains(['\n', '\r'])
    {
        return Err(Error::Field("release GitHub repository/tag".into()));
    }
    Ok(())
}
