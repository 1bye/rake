//! macOS metadata policy and native release finalization.
mod finalize;
pub use finalize::{notarize, package, preflight, sign_app, verify_artifact};
mod loaders;
pub use loaders::{normalize_loaders, signable_inventory};
use std::path::Path;

/// macOS metadata/finalization failures.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Native filesystem/process failure.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Native payload or tool failed validation.
    #[error("macOS distribution: {0}")]
    Native(String),
    /// Malformed custom plist.
    #[error(transparent)]
    Plist(#[from] plist::Error),
    /// An override disagrees with the validated application identity.
    #[error("custom plist conflicts with generated {0}")]
    Identity(&'static str),
}

/// Reads a custom plist and protects generated executable and application identity.
/// # Errors
/// Rejects malformed plists and reserved fields with conflicting values.
pub fn info_plist(
    path: &Path,
    identifier: &str,
    executable: &str,
    version: &str,
    build: &str,
) -> Result<plist::Value, Error> {
    let value = plist::Value::from_file(path)?;
    let dict = value.as_dictionary().ok_or(Error::Identity("dictionary"))?;
    for (key, expected) in [
        ("CFBundleIdentifier", identifier),
        ("CFBundleExecutable", executable),
        ("CFBundlePackageType", "APPL"),
        ("CFBundleShortVersionString", version),
        ("CFBundleVersion", build),
    ] {
        if let Some(value) = dict.get(key)
            && value.as_string() != Some(expected)
        {
            return Err(Error::Identity(key));
        }
    }
    Ok(value)
}

/// Adds guarded Sparkle metadata while rejecting conflicting custom trust settings.
/// # Errors
/// Rejects malformed public keys, non-dictionary plists and conflicting updater fields.
pub fn sparkle_info_plist(
    mut value: plist::Value,
    feed: &str,
    public_key: &str,
) -> Result<plist::Value, Error> {
    use base64::Engine as _;
    let key = base64::engine::general_purpose::STANDARD
        .decode(public_key.trim())
        .map_err(|error| Error::Native(error.to_string()))?;
    if key.len() != 32 {
        return Err(Error::Identity("SUPublicEDKey"));
    }
    let dictionary = value
        .as_dictionary_mut()
        .ok_or(Error::Identity("dictionary"))?;
    for (name, expected) in [
        ("SUFeedURL", plist::Value::String(feed.into())),
        (
            "SUPublicEDKey",
            plist::Value::String(public_key.trim().into()),
        ),
        ("SUAllowsAutomaticUpdates", plist::Value::Boolean(false)),
        (
            "SUVerifyUpdateBeforeExtraction",
            plist::Value::Boolean(true),
        ),
        ("SURequireSignedFeed", plist::Value::Boolean(true)),
    ] {
        if dictionary.get(name).is_some_and(|value| value != &expected) {
            return Err(Error::Identity(name));
        }
        dictionary.insert(name.into(), expected);
    }
    Ok(value)
}

/// Returns an empty custom plist for generated platform integration metadata.
pub fn empty_info_plist() -> plist::Value {
    plist::Value::Dictionary(plist::Dictionary::new())
}
