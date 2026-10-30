//! macOS metadata policy and native release finalization.
use std::path::Path;

/// macOS metadata/finalization failures.
#[derive(Debug, thiserror::Error)]
pub enum Error {
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
