//! Routing identities, outcomes, snapshots and guard contracts.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::{RouteAddress, RouteDecodeError};

/// One live runtime; a full browser reload or native process restart creates a new ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RuntimeId(
    /// Underlying runtime UUID.
    pub uuid::Uuid,
);

impl fmt::Display for RuntimeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

macro_rules! identity {
    ($name:ident, $prefix:literal, $doc:literal) => {
        #[doc = $doc]
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        pub struct $name(
            /// Numeric identity scoped to its runtime; zero is reserved for internal sentinels.
            pub u64,
        );

        impl $name {
            /// Returns the numeric value, scoped by runtime identity.
            pub fn value(self) -> u64 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, concat!($prefix, "-{}"), self.0)
            }
        }

        impl FromStr for $name {
            type Err = ParseIdError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                let number = value
                    .strip_prefix(concat!($prefix, "-"))
                    .ok_or(ParseIdError)?;
                let number = number.parse::<u64>().map_err(|_| ParseIdError)?;

                if number == 0 {
                    return Err(ParseIdError);
                }

                Ok(Self(number))
            }
        }
    };
}

identity!(WindowId, "window", "One UI host within a runtime.");

identity!(
    TabId,
    "tab",
    "One independent application tab within a runtime."
);

identity!(
    EntryId,
    "entry",
    "One retained screen instance within a tab history."
);

identity!(
    OperationId,
    "op",
    "One eagerly submitted application operation."
);

identity!(
    DecisionId,
    "decision",
    "One revision-specific request for an explicit choice."
);

identity!(
    TaskId,
    "task",
    "One task or activity registration within a runtime."
);

identity!(
    RunId,
    "run",
    "One fresh execution of managed work within a runtime."
);

identity!(
    GenerationId,
    "generation",
    "One accepted code generation within a runtime."
);

/// Invalid external identity spelling.
#[derive(Clone, Debug, thiserror::Error)]
#[error("invalid identity: expected the correct prefix and a positive integer")]
pub struct ParseIdError;

/// Requested application host target. NewTab always means an internal application tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpenTarget {
    /// Push into the concrete tab resolved at submission.
    CurrentTab,
    /// Always create a new, independent tab and root entry.
    NewTab,
    /// Creates a fresh host window when the runtime advertises that capability.
    NewWindow,
}

/// Whether creation changes the displayed tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Activation {
    /// Select the created tab.
    Activate,
    /// Keep the current selection, or select the first tab in an empty workspace.
    PreserveSelection,
}

/// Eligibility policy for the current entry's managed activity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TabMode {
    /// Managed activity follows presentation.
    #[default]
    Foreground,
    /// Current-entry managed activity remains eligible when hidden.
    Live,
}

/// Explicit creation and activation options.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenOptions {
    /// Application host target.
    pub target: OpenTarget,
    /// Selection policy.
    pub activation: Activation,
    /// New-tab mode only. Existing modes are changed through `set_mode`.
    pub mode: Option<TabMode>,
}

impl OpenOptions {
    /// Creates and selects a fresh application tab.
    pub fn new_tab() -> Self {
        Self {
            target: OpenTarget::NewTab,
            activation: Activation::Activate,
            mode: None,
        }
    }

    /// Requests a new host window. Unsupported capabilities are never downgraded.
    pub fn new_window() -> Self {
        Self {
            target: OpenTarget::NewWindow,
            ..Self::new_tab()
        }
    }

    /// Sets selection policy independently of work mode.
    pub fn activation(mut self, activation: Activation) -> Self {
        self.activation = activation;

        self
    }

    /// Sets a new tab's work mode.
    pub fn mode(mut self, mode: TabMode) -> Self {
        self.mode = Some(mode);

        self
    }
}

/// Semantic reuse scope for current entries across retained windows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RevealScope {
    /// Match within the explicitly bound window.
    Window,
    /// Match any advertised host in this runtime.
    #[default]
    Application,
}

/// Concrete identity of a committed entry. Data loading and frame presentation are separate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Opened {
    /// Host owning the tab.
    pub window_id: WindowId,
    /// Independent tab session.
    pub tab_id: TabId,
    /// Retained screen instance.
    pub entry_id: EntryId,
}

/// Identity after a push, replacement or history traversal.
pub type Entered = Opened;
/// Identity after selection without adding history.
pub type Selected = Opened;

/// Result of semantic reuse or fallback creation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revealed {
    /// Selected matching or newly created entry.
    pub opened: Opened,
    /// Whether an existing current entry was reused.
    pub reused: bool,
}

/// A committed tab closure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Closed {
    /// Disposed tab.
    pub tab_id: TabId,
}

/// A committed explicit policy change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModeChanged {
    /// Updated tab.
    pub tab_id: TabId,
    /// Stored mode.
    pub mode: TabMode,
}

/// Successful commit, explicit cancellation or deliberate blocking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome<T> {
    /// Router state and the prepared screen were attached atomically.
    Committed(T),
    /// A user or newer browser traversal cancelled the transition.
    Cancelled(CancelReason),
    /// A guard or retention limit blocked the transition.
    Blocked(BlockReason),
}

/// Reason a transition was explicitly cancelled.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CancelReason {
    /// Explicit cancellation was accepted before commit.
    Requested,
    /// A structured decision selected cancellation.
    Decision,
    /// A newer browser traversal superseded this one.
    Superseded,
}

/// An application-supplied, human-readable guard reason.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardReason(pub String);

/// Deliberate rejection distinct from a technical failure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockReason {
    /// An access/disposal guard denied the change.
    Guard(GuardReason),
    /// The configured history capacity would be exceeded.
    HistoryCapacity,
    /// The tab cursor has no entry in that direction.
    HistoryBoundary,
}

/// A screen factory could not construct a usable route view.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[error("route mount failed: {0}")]
pub struct MountError(pub String);

/// Technical operation failures; intentional blocking is an `Outcome` instead.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum OperationError {
    /// Guarded quit has stopped admission to this operation.
    #[error("the application is quitting")]
    Quitting,
    /// Development reload has paused navigation and application commands.
    #[error("runtime is paused for reload")]
    Reloading,
    /// Registered application handler failure.
    #[error(transparent)]
    Command(#[from] crate::control::CommandError),
    /// Invalid option combination.
    #[error("invalid navigation options")]
    InvalidOptions,
    /// The requested host capability is not implemented.
    #[error("unsupported navigation target: {0:?}")]
    UnsupportedCapability(OpenTarget),
    /// A concrete tab or entry no longer exists.
    #[error("the navigation target has closed")]
    ClosedTarget,
    /// CurrentTab was submitted without a selected or bound tab.
    #[error("no current tab is available")]
    NoCurrentTab,
    /// A guarded window close has stopped admission in this host.
    #[error("the requested window is closing")]
    WindowClosing,
    /// The navigator names a host that is not available.
    #[error("the requested window is unavailable")]
    WindowUnavailable,
    /// Malformed or unavailable destination.
    #[error(transparent)]
    Route(#[from] RouteDecodeError),
    /// View creation failed before commit; existing state remains.
    #[error(transparent)]
    Mount(#[from] MountError),
    /// The runtime was disposed or shut down.
    #[error("the routing runtime has shut down")]
    RuntimeShutdown,
    /// Another driver already owns this router.
    #[error("the router already has an operation driver")]
    AlreadyDriving,
    /// Pending-operation admission limit was reached.
    #[error("the router's operation queue is full")]
    QueueFull,
    /// A formerly submitted operation was evicted from the bounded result registry.
    #[error("the operation's retained status has expired")]
    OperationExpired,
    /// No such operation was submitted to this runtime.
    #[error("unknown operation")]
    UnknownOperation,
    /// Invalid choice schema returned by an application guard.
    #[error("invalid guard decision: {0}")]
    InvalidDecision(String),
}

/// Observable operation state, independent of polling the observation handle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationState {
    /// Accepted into the bounded queue.
    Queued,
    /// Planning, guarding or preparing a screen.
    Running,
    /// Waiting for an explicit decision ID.
    AwaitingDecision(DecisionId),
    /// State change committed.
    Committed,
    /// Explicit cancellation.
    Cancelled(CancelReason),
    /// Deliberate rejection.
    Blocked(BlockReason),
    /// Technical failure.
    Failed(OperationError),
}

/// Why retained state would be destroyed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DisposeReason {
    /// Dispose every retained entry in an approved window close.
    CloseWindow,
    /// Dispose all entries in a closing tab.
    CloseTab,
    /// Replace the current instance with a fresh one.
    ReplaceEntry,
    /// Discard entries after a backward cursor when pushing a new destination.
    TruncateForwardHistory,
    /// Explicit future retention eviction.
    EvictEntry,
    /// Dispose every retained entry during approved application quit.
    QuitApplication,
}

/// Guard category, separating access from destruction.
#[derive(Clone, Copy, Debug)]
pub enum GuardKind {
    /// Enter or reveal a destination, without destroying the outgoing entry.
    Enter,
    /// Destroy a retained instance.
    Dispose(DisposeReason),
}

/// The disposition after a decision's optional preparation succeeds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChoiceAction {
    /// Authorize the evaluated state revision.
    Proceed,
    /// Cancel the transition.
    Cancel,
}

/// One stable external choice ID and its display label.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChoiceDescriptor {
    /// Stable machine-readable choice ID.
    pub id: String,
    /// Human-readable label.
    pub label: String,
    /// Explicit effect after preparation.
    pub action: ChoiceAction,
}

/// A pending decision's owned external representation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DecisionSnapshot {
    /// Host presenting this decision. Before a new window is created, access
    /// decisions appear in its originating host (or another surviving host).
    #[serde(default = "initial_window")]
    pub window_id: WindowId,
    /// Revision-specific decision ID.
    pub id: DecisionId,
    /// Submitted operation waiting on this decision.
    pub operation_id: OperationId,
    /// Screen whose state was evaluated.
    pub entry_id: EntryId,
    /// Human-readable prompt.
    pub message: String,
    /// Stable choice descriptors without executable callbacks.
    pub choices: Vec<ChoiceDescriptor>,
}

/// Invalid or obsolete decision responses never authorize another transition.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum DecisionError {
    /// The decision expired or its entry changed.
    #[error("the decision is obsolete")]
    Obsolete,
    /// The named choice is not part of this decision.
    #[error("unknown decision choice")]
    UnknownChoice,
}

/// One owned history entry description; view resources remain runtime-owned.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EntrySnapshot {
    /// Stable retained-entry ID.
    pub id: EntryId,
    /// Canonical owned address.
    pub destination: RouteAddress,
    /// Whether application state is currently marked dirty.
    pub dirty: bool,
    /// Entry state revision, distinct from the workspace revision.
    pub revision: u64,
}

/// One independent tab and its retained history.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TabSnapshot {
    /// Host that owns this tab and its retained history.
    #[serde(default = "initial_window")]
    pub window_id: WindowId,
    /// Tab identity.
    pub id: TabId,
    /// Managed current-entry activity policy.
    pub mode: TabMode,
    /// Ordered retained screen instances.
    pub entries: Vec<EntrySnapshot>,
    /// Index of the current retained instance.
    pub cursor: usize,
}

/// An owned authoritative routing snapshot; tracing loss cannot corrupt it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RouterSnapshot {
    /// Guarded application quit, including running-job coordination.
    #[serde(default)]
    pub quit: crate::quit::QuitStatus,
    /// Development boundary and compatibility outcome.
    pub reload: crate::reload::ReloadStatus,
    /// Managed work and observed host visibility, independent of tracing loss.
    pub work: crate::tasks::WorkSnapshot,
    /// Explicitly supported application targets in this runtime.
    pub supported_targets: Vec<OpenTarget>,
    /// One running host lifetime.
    pub runtime_id: RuntimeId,
    /// Host used by the legacy selected-tab field (initially window-1).
    pub window_id: WindowId,
    /// Independent host selections and close operations.
    #[serde(default)]
    pub windows: Vec<WindowSnapshot>,
    /// Revision for `Router::changed` observation.
    pub revision: u64,
    /// Tabs in creation order.
    pub tabs: Vec<TabSnapshot>,
    /// Displayed tab, if any.
    pub selected: Option<TabId>,
    /// Pending structured decisions.
    pub decisions: Vec<DecisionSnapshot>,
    /// Retained operation statuses, including all pending operations.
    pub operations: Vec<(OperationId, OperationState)>,
    /// Configured history limit; no implicit state eviction occurs.
    pub history_capacity: Option<usize>,
    /// Browser synchronization failure after an otherwise successful router commit.
    pub history_error: Option<String>,
}

/// One host and its independent selected tab. Tab histories are in the router snapshot.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WindowSnapshot {
    /// Stable runtime-scoped identity.
    pub id: WindowId,
    /// Independently displayed tab, or None for an empty host.
    pub selected: Option<TabId>,
    /// Pending guarded close, including while waiting for a decision.
    pub closing: Option<OperationId>,
    /// Observed visibility of this host, independently of keyboard focus.
    pub visibility: crate::tasks::HostVisibility,
}

/// Committed disposal of one host and all its retained tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowClosed {
    /// The disposed host; other windows and application-owned work remain alive.
    pub window_id: WindowId,
}

fn initial_window() -> WindowId {
    WindowId(1)
}
