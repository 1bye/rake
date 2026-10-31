//! Signed full-artifact feed shared by AppImage runtime verification and publishers.
use serde::{Deserialize, Serialize};

/// Exact signed payload bytes are retained as a string to avoid JSON reserialization ambiguity.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedUpdateFeed {
    /// JSON-encoded [`UpdateFeed`] signed verbatim as UTF-8.
    pub payload: String,
    /// Ed25519 signature encoded as 128 hexadecimal characters.
    pub signature: String,
}

/// An authenticated channel inventory, including older installed versions for ownership verification.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateFeed {
    /// Feed contract revision.
    pub schema_version: u32,
    /// Bound application identifier.
    pub identifier: String,
    /// Bound target triple.
    pub target: String,
    /// Bound channel.
    pub channel: String,
    /// Publication time, Unix seconds.
    pub published_at: u64,
    /// Expiry time, Unix seconds; expired feeds cannot authorize installation.
    pub expires_at: u64,
    /// Full image releases retained for update and installed-image verification.
    pub releases: Vec<UpdateRelease>,
}

/// One complete immutable application image; this protocol has no custom delta format.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateRelease {
    /// Cargo application version.
    pub version: String,
    /// Native package/update version.
    pub package_version: String,
    /// Absolute public HTTPS artifact URL.
    pub url: String,
    /// Complete artifact SHA-256 encoded as 64 hexadecimal characters.
    pub sha256: String,
    /// Exact artifact size, bounded before and during download.
    pub size: u64,
}

/// Detached signature sidecar for an otherwise unchanged native provider feed.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeFeedSignature {
    /// Sidecar contract revision.
    pub schema_version: u32,
    /// Publication time, Unix seconds.
    pub published_at: u64,
    /// Expiry time, Unix seconds.
    pub expires_at: u64,
    /// Context-bound Ed25519 signature encoded as hexadecimal.
    pub signature: String,
}

impl NativeFeedSignature {
    /// Returns exact signing bytes bound to provider, identity, target, channel and freshness.
    pub fn message(
        &self,
        provider: &str,
        identifier: &str,
        target: &str,
        channel: &str,
        feed: &[u8],
    ) -> Vec<u8> {
        let mut message = format!(
            "rake-native-feed-v1\n{provider}\n{identifier}\n{target}\n{channel}\n{}\n{}\n",
            self.published_at, self.expires_at
        )
        .into_bytes();

        message.extend_from_slice(feed);

        message
    }
}
