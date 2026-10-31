//! Versioned extension contracts keep private credentials and engine types out of plugins.
use crate::{Format, Platform};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

/// Explicit external format, applied to one already-built native input format.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct CustomFormat {
    /// Globally unique slug used in CLI reports and the output directory.
    pub name: String,
    /// Native host on which this backend runs.
    pub platform: Platform,
    /// Built-in native artifact supplied as input (for example a signed app).
    pub input_format: Format,
    /// Explicit argv; no shell expansion or format-derived executable discovery.
    pub command: Vec<String>,
    /// Explicit independent native verifier. Without it, output remains local
    /// and unverified and cannot be promoted by release publication.
    #[serde(default)]
    pub verify_command: Option<Vec<String>>,
}

/// Public, serializable native artifact input; its paths refer to owned staging.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendRequest {
    /// Contract revision, currently 1.
    pub schema_version: u32,
    /// Custom backend name from validated configuration.
    pub name: String,
    /// Stable application identity.
    pub identifier: String,
    /// Cargo application version.
    pub version: String,
    /// Exact Rust target.
    pub target: String,
    /// Copy of a completed built-in artifact, confined to this operation's staging.
    pub input: PathBuf,
    /// Dedicated empty output directory owned by the coordinator.
    pub output: PathBuf,
    /// Public signing/tool profile references, never serialized private keys.
    pub tool_references: BTreeMap<String, String>,
}

/// A backend only declares paths; it cannot assert native signing or notarization.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendOutput {
    /// Contract revision.
    pub schema_version: u32,
    /// Paths relative to BackendRequest.output. The coordinator validates every path/hash.
    pub artifacts: Vec<PathBuf>,
}

/// Private runtime/component preparation inputs, independent of a specific recipe.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeRequest {
    /// Contract revision, currently 1.
    pub schema_version: u32,
    /// Runtime/component identity.
    pub name: String,
    /// Exact Rust target.
    pub target: String,
    /// Confined copy of the declared local/locked component source.
    pub input: PathBuf,
    /// Dedicated owned output directory.
    pub output: PathBuf,
    /// Public recipe-specific options.
    pub options: BTreeMap<String, String>,
}

/// A prepared runtime tree and its declared native-code inventory.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedComponent {
    /// Contract revision.
    pub schema_version: u32,
    /// Runtime tree relative to RecipeRequest.output.
    pub root: PathBuf,
    /// Every native binary, relative to root; the coordinator checks actual code/architecture.
    pub native_code: Vec<PathBuf>,
}

/// Independent native verification request, bound to exact output bytes.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifyRequest {
    /// Contract revision.
    pub schema_version: u32,
    /// Backend identity.
    pub name: String,
    /// Application identity.
    pub identifier: String,
    /// Exact target triple.
    pub target: String,
    /// Expected Cargo application version.
    pub version: String,
    /// Expected public native signing/tool profile references.
    pub tool_references: BTreeMap<String, String>,
    /// Confined completed artifact path.
    pub artifact: PathBuf,
    /// Coordinator-measured SHA-256.
    pub sha256: String,
}

/// Native verifier evidence; backend declarations never deserialize into this type.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verification {
    /// Contract revision.
    pub schema_version: u32,
    /// Exact artifact hash accepted by the native verifier.
    pub sha256: String,
    /// Native verifier's signature/format policy (for example an enterprise installer policy).
    pub policy: String,
}

/// Errors returned by explicitly registered extension implementations.
#[derive(Debug, thiserror::Error)]
pub enum ExtensionError {
    /// A request or returned path violates the extension contract.
    #[error("extension contract: {0}")]
    Contract(String),
    /// An explicitly selected native extension tool failed.
    #[error("extension tool: {0}")]
    Tool(String),
    /// Filesystem or process failure.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Library consumers can register a concrete package producer without importing an engine.
/// The owner supplies isolated staging; returned paths are untrusted declarations
/// until checked by the coordinator. Implementations must finish owned native work.
pub trait PackageBackend {
    /// Produces declared output, without granting native trust to it.
    /// # Errors
    /// Returns backend/tool failures; the coordinator retains its previous output.
    fn package(&self, request: &BackendRequest) -> Result<BackendOutput, ExtensionError>;
}

/// Library extension point for real component preparation implementations.
pub trait ComponentRecipe {
    /// Prepares a runtime inside the provided owned output directory.
    /// # Errors
    /// Returns source/target/tool failures; never mutates the original source/cache.
    fn prepare(&self, request: &RecipeRequest) -> Result<PreparedComponent, ExtensionError>;
}
