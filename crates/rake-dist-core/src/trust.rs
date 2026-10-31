//! Public signing references; private keys remain in native stores.
use serde::{Deserialize, Serialize};

/// Optional local signing overrides supplied by the release command.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct TrustOptions {
    /// Enforce production trust prerequisites rather than producing a development artifact.
    pub required: bool,
    /// Existing macOS codesign identity name or fingerprint.
    pub macos_identity: Option<String>,
    /// Existing notarytool Keychain profile name.
    pub notary_profile: Option<String>,
    /// Existing Windows signing certificate thumbprint in the current-user store.
    pub windows_thumbprint: Option<String>,
}

/// Native verifier results for the final artifact.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TrustRecord {
    /// Verified signature kind; empty for an unsigned development artifact.
    pub signature: Option<String>,
    /// An RFC 3161 timestamp was requested and native verification accepted it.
    #[serde(default)]
    pub timestamped: bool,
    /// Apple accepted this payload for notarization.
    pub notarized: bool,
    /// The artifact's stapled Apple ticket was validated.
    pub stapled: bool,
}
