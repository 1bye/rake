//! Managed-work data and inspectable failure contracts.

use crate::router::{EntryId, RuntimeId, TabId, WindowId};
pub use crate::router::{GenerationId, RunId, TaskId};
use serde::{Deserialize, Serialize};

/// Managed-work admission, event delivery and diagnostic retention bounds.
#[derive(Clone, Debug)]
pub struct TaskConfig {
    /// Maximum registered activities and retained task records combined.
    pub capacity: usize,
    /// Most recent terminal records retained, excluding still-registered factories.
    pub completed: usize,
    /// Maximum publications awaiting handler delivery. Overflow is an explicit error.
    pub publications: usize,
}

impl Default for TaskConfig {
    fn default() -> Self {
        Self {
            capacity: 1024,
            completed: 128,
            publications: 256,
        }
    }
}

/// Observed visibility, independent of keyboard focus and app-tab selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostVisibility {
    /// Host has no visibility signal; eligibility assumes the host is visible.
    #[default]
    Unknown,
    /// Host reports that its document or native surface is visible.
    Visible,
    /// Host reports a hidden document, minimized or occluded native surface.
    Hidden,
}

/// Lifetime that owns a task or registered activity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskOwner {
    /// Entire application runtime.
    Application,
    /// One tab, across entry navigation and selection changes.
    Tab(TabId),
    /// One retained screen instance.
    Entry(EntryId),
}

/// Immutable causal origin, retained even when an application task outlives its page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskOrigin {
    /// Originating runtime.
    pub runtime_id: RuntimeId,
    /// Originating host.
    pub window_id: WindowId,
    /// Originating tab, when submitted from a page or tab.
    pub tab_id: Option<TabId>,
    /// Originating entry, when submitted from a page.
    pub entry_id: Option<EntryId>,
}

/// Work eligibility and restart contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskKind {
    /// One future; never automatically restarted.
    Task,
    /// Fresh futures follow the tab's Foreground/Live policy.
    Activity,
    /// Fresh futures require app presentation and host visibility in either tab mode.
    WhileVisible,
}

/// Latest run's state; retained factories can start another run after eligibility returns.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskState {
    /// Factory registered but not yet eligible.
    Registered,
    /// Eagerly admitted, awaiting a driver poll.
    Queued,
    /// Future currently being driven.
    Running,
    /// Cancellation requested; publication already rejected.
    Cancelling,
    /// Future completed successfully.
    Completed,
    /// Cancelled future was dropped.
    Cancelled,
    /// Future or event handler failed.
    Failed(TaskError),
}

/// Inspectable managed-work failures.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum TaskError {
    /// Development reload has paused one-shot admission.
    #[error("managed work is paused for reload")]
    Reloading,
    /// Owner or runtime no longer accepts work.
    #[error("work owner has been disposed")]
    OwnerDisposed,
    /// Run was cancelled, restarted, or belongs to an unaccepted generation.
    #[error("activity run is no longer current")]
    StaleRun,
    /// Labels and resource keys must be nonempty.
    #[error("work label must not be empty")]
    InvalidLabel,
    /// Each entry has one factory per resource key.
    #[error("an activity with this resource key is already registered")]
    DuplicateResource,
    /// Configured task record admission bound reached.
    #[error("managed-work capacity reached")]
    Capacity,
    /// Configured publication queue bound reached.
    #[error("publication queue is full")]
    PublicationQueueFull,
    /// A publication has no typed handler in this scope.
    #[error("no handler is registered for this event type")]
    UnregisteredEvent,
    /// A terminal work record was removed by retention bounds.
    #[error("task record has expired")]
    Expired,
    /// Application I/O or domain work failed.
    #[error("{0}")]
    Failed(String),
}

/// Owned diagnostic record; snapshots never retain a future, factory or view.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskSnapshot {
    /// Stable task or activity registration identity.
    pub id: TaskId,
    /// Application label; for activities this is the resource key.
    pub key: String,
    /// Lifetime owner.
    pub owner: TaskOwner,
    /// Original causal context.
    pub origin: TaskOrigin,
    /// One-shot, policy activity or visibility activity.
    pub kind: TaskKind,
    /// Latest run identity; changes on every restart.
    pub run_id: Option<RunId>,
    /// Accepted code generation. Initial runtimes currently use generation 1.
    pub generation_id: GenerationId,
    /// Whether an activity's factory is currently eligible.
    pub eligible: bool,
    /// Latest run outcome or progress.
    pub state: TaskState,
}

/// Authoritative work state, separate from bounded tracing output.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkSnapshot {
    /// Revision for `Router::changed` observation.
    pub revision: u64,
    /// Host visibility; Unknown is reported honestly on unsupported native backends.
    pub host_visibility: HostVisibility,
    /// Retained active registrations and recent terminal work.
    pub tasks: Vec<TaskSnapshot>,
    /// Publications awaiting dispatch.
    pub pending_publications: usize,
}
