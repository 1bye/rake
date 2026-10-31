//! Development checkpoints and reload coordination, separate from durable sessions.
//!
//! Checkpoints contain owned data only. Native code replacement is performed by an
//! external reloader, which must retain old code until all its objects are retired.

use crate::router::{EntryId, GenerationId, RouteAddress, RuntimeId, TabId, TabMode, WindowId};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// Maximum development checkpoint size, leaving room in the control envelope.
pub const MAX_CHECKPOINT_BYTES: usize = 512 * 1024;

/// Explicitly versioned application data; never a serialized entity or callback.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    /// Stable application-defined schema name.
    pub schema: String,
    /// Application-defined migration version.
    pub version: u32,
    /// Owned, portable state.
    pub data: serde_json::Value,
}

/// Opt-in state for one retained entry. Recreate services and resources during mount.
/// Override `restore` to migrate an older version explicitly.
pub trait ReloadState: Serialize + DeserializeOwned {
    /// Stable identity across compatible builds.
    const SCHEMA: &'static str;
    /// Current state version.
    const VERSION: u32;

    /// Captures owned data. Called synchronously at a quiescent reload boundary.
    ///
    /// # Errors
    /// Returns [`ReloadError::State`] if application data cannot be serialized.
    fn checkpoint(&self) -> Result<Checkpoint, ReloadError> {
        Ok(Checkpoint {
            schema: Self::SCHEMA.into(),
            version: Self::VERSION,
            data: serde_json::to_value(self)
                .map_err(|error| ReloadError::State(error.to_string()))?,
        })
    }

    /// Validates compatibility before decoding application state.
    ///
    /// # Errors
    /// Returns [`ReloadError::Incompatible`] for a schema/version mismatch, or
    /// [`ReloadError::State`] if the checkpoint data cannot be decoded.
    fn restore(checkpoint: Checkpoint) -> Result<Self, ReloadError> {
        if checkpoint.schema != Self::SCHEMA || checkpoint.version != Self::VERSION {
            return Err(ReloadError::Incompatible(format!(
                "{} version {} cannot restore {} version {}",
                Self::SCHEMA,
                Self::VERSION,
                checkpoint.schema,
                checkpoint.version
            )));
        }

        serde_json::from_value(checkpoint.data)
            .map_err(|error| ReloadError::State(error.to_string()))
    }
}

/// One retained history entry, independent of the current view implementation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryCheckpoint {
    /// Logical identity, preserved across compatible restoration.
    pub id: EntryId,
    /// Typed route address validated by the replacement route codec.
    pub destination: RouteAddress,
    /// Unsaved state indicator.
    pub dirty: bool,
    /// Application revision.
    pub revision: u64,
    /// Explicitly supported view state.
    pub state: Option<Checkpoint>,
}

/// Independent tab and its complete retained history.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabCheckpoint {
    /// Owning native/Web host; omitted version-1 checkpoints belong to window-1.
    #[serde(default = "initial_window")]
    pub window_id: WindowId,
    /// Logical identity.
    pub id: TabId,
    /// Managed work policy.
    pub mode: TabMode,
    /// Current history position.
    pub cursor: usize,
    /// Selection ordering, used by reveal.
    pub selected_at: u64,
    /// Retained entries in history order.
    pub entries: Vec<EntryCheckpoint>,
}

/// A development workspace, not a durable user session or a transaction of external effects.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceCheckpoint {
    /// Per-window selection; empty only in legacy version-1 envelopes.
    #[serde(default)]
    pub windows: Vec<WindowCheckpoint>,
    /// Envelope version; current captures use 2, with version 1 accepted on restore.
    pub version: u32,
    /// Prevents loading another application's development state.
    pub application_id: String,
    /// Original host lifetime. A full restart always receives a new RuntimeId.
    pub source_runtime: RuntimeId,
    /// Generation being retired.
    pub generation: GenerationId,
    /// High water mark so restored factory runs receive fresh identities.
    pub run_sequence: u64,
    /// Selected tab, or None for an empty workspace.
    pub selected: Option<TabId>,
    /// All retained tabs and histories.
    pub tabs: Vec<TabCheckpoint>,
}

/// Actual replacement mechanism, reported to clients instead of implying hot swapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReloadMechanism {
    /// Checkpointed process restart; changes RuntimeId.
    NativeRestart,
    /// Full page reload into rebuilt Wasm; changes RuntimeId.
    WebPageReload,
    /// External native backend preserves the host and owns code lifetime safety.
    NativeCodeReplacement,
}

/// Preparation request with a stable supervisor-generated identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReloadRequest {
    /// Unique boundary identity, 1–128 bytes.
    pub id: String,
    /// Actual replacement strategy.
    pub mechanism: ReloadMechanism,
}

/// Observable state of the development reload boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReloadPhase {
    /// Normal execution.
    Idle,
    /// Old publications rejected; cancellation and UI detachment are in progress.
    Quiescing,
    /// Safe to capture logical state, subject to application checkpoint errors.
    Ready,
    /// Replacement is reconstructing provisional entries.
    Restoring,
    /// Compatible replacement attached successfully.
    Restored,
    /// Explicit abort resumed eligible old factories with fresh run IDs.
    Aborted,
    /// State retained for retry or explicit restart.
    RestoreFailed,
}

/// Owned status available through native and browser control adapters.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReloadStatus {
    /// Previous host whose workspace was restored; lets observers follow a replacement
    /// without guessing between independent instances of the same application.
    #[serde(default)]
    pub restored_from: Option<RuntimeId>,
    /// Current code generation; increases on compatible replacement.
    pub generation: GenerationId,
    /// Current boundary phase.
    pub phase: ReloadPhase,
    /// Boundary being prepared, if any.
    pub request: Option<ReloadRequest>,
    /// Failure requiring an explicit recovery choice.
    pub error: Option<String>,
}

/// Inspectable preparation, compatibility and restoration failures.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum ReloadError {
    /// Navigation/commands must settle before entering a boundary.
    #[error("runtime is busy; finish pending operations and decisions before reloading")]
    Busy,
    /// Work or the UI callback boundary has not drained.
    #[error("reload is not quiescent")]
    NotReady,
    /// A stale supervisor tried to finish a different boundary.
    #[error("reload request does not match the prepared boundary")]
    WrongRequest,
    /// State with unsaved changes has no explicit capture hook.
    #[error("dirty entry {0} has no reload state provider")]
    MissingState(EntryId),
    /// Invalid envelope, identities, route or policy.
    #[error("incompatible reload checkpoint: {0}")]
    Incompatible(String),
    /// Application checkpoint/restore failure.
    #[error("reload state: {0}")]
    State(String),
    /// Checkpoints are bounded like the control transport.
    #[error("reload checkpoint exceeds 512 KiB")]
    TooLarge,
}

/// A host's identity and independently selected retained tab during development reload.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowCheckpoint {
    /// Stable logical host identity.
    pub id: WindowId,
    /// Selected tab, or None for an empty host.
    pub selected: Option<TabId>,
}

fn initial_window() -> WindowId {
    WindowId(1)
}
