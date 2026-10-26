//! Routing identities, outcomes, snapshots and guard contracts.

use std::fmt;
use std::rc::Rc;
use std::str::FromStr;

use futures::future::{LocalBoxFuture, Shared};
use serde::{Deserialize, Serialize};

use super::{Route, RouteAddress, RouteDecodeError};

/// One live runtime; a full browser reload or native process restart creates a new ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RuntimeId(pub(crate) uuid::Uuid);

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
        pub struct $name(pub(crate) u64);

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

/// Router bounds. History is unlimited by default; entries are never silently evicted.
#[derive(Clone, Debug)]
pub struct RouterConfig {
    /// Lifetime request-key admission bound for retry-safe runtime control.
    pub control_requests: usize,
    /// Managed-work admission, publication and retention bounds.
    pub tasks: crate::tasks::TaskConfig,
    /// Maximum entries per tab. Capacity blocks a push instead of discarding state.
    pub history_capacity: Option<usize>,
    /// Number of most recently completed operations retained for status queries.
    pub completed_operations: usize,
    /// Upper bound on queued and executing operations.
    pub pending_operations: usize,
    /// Initial work policy for fresh tabs.
    pub default_mode: TabMode,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            control_requests: 1024,
            tasks: crate::tasks::TaskConfig::default(),
            history_capacity: None,
            completed_operations: 256,
            pending_operations: 1024,
            default_mode: TabMode::Foreground,
        }
    }
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

/// An eagerly submitted operation. Dropping it never cancels the command.
#[derive(Clone)]
#[must_use = "observe the handle or retain its ID to inspect the operation outcome"]
pub struct Operation<T: Clone> {
    pub(crate) id: OperationId,
    pub(crate) result: Shared<LocalBoxFuture<'static, Result<Outcome<T>, OperationError>>>,
}

impl<T: Clone> Operation<T> {
    /// Returns its ID immediately, before execution or guard decisions complete.
    pub fn id(&self) -> OperationId {
        self.id
    }

    /// Observes the eventual outcome. Submission does not depend on calling this method.
    pub async fn wait(self) -> Result<Outcome<T>, OperationError> {
        self.result.await
    }
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

/// An owned guard input. No UI context is borrowed across its async work.
#[derive(Clone)]
pub struct GuardContext<R: Route> {
    /// Concrete host of the guarded entry, captured independently of keyboard focus.
    pub window_id: WindowId,
    /// Submitted operation.
    pub operation_id: OperationId,
    /// Concrete tab, resolved before guarding.
    pub tab_id: TabId,
    /// Screen being entered or destroyed.
    pub entry_id: EntryId,
    /// Compact destination value.
    pub destination: R,
    /// Dirty state at this revision.
    pub dirty: bool,
    /// State revision requiring revalidation after async work.
    pub revision: u64,
    /// Access or disposal purpose.
    pub kind: GuardKind,
}

/// Guard result shared by UI decisions and future external runtime clients.
pub enum GuardResult {
    /// Permit this revision of the transition.
    Allow,
    /// Deliberately block it.
    Deny(GuardReason),
    /// Request an explicit structured choice.
    Ask(DecisionRequest),
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

pub(crate) type Preparation = Rc<dyn Fn() -> LocalBoxFuture<'static, Result<(), GuardReason>>>;

/// An executable registered choice. Preparations may save externally; they are not rolled back.
#[derive(Clone)]
pub struct DecisionChoice {
    pub(crate) descriptor: ChoiceDescriptor,
    pub(crate) prepare: Option<Preparation>,
}

impl DecisionChoice {
    /// A choice that proceeds without preparation, such as discard.
    pub fn proceed(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            descriptor: ChoiceDescriptor {
                id: id.into(),
                label: label.into(),
                action: ChoiceAction::Proceed,
            },
            prepare: None,
        }
    }

    /// A choice that explicitly cancels the operation.
    pub fn cancel(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            descriptor: ChoiceDescriptor {
                id: id.into(),
                label: label.into(),
                action: ChoiceAction::Cancel,
            },
            prepare: None,
        }
    }

    /// Registers local async preparation, such as saving, before authorizing a transition.
    pub fn prepare<F, Fut>(mut self, prepare: F) -> Self
    where
        F: Fn() -> Fut + 'static,
        Fut: std::future::Future<Output = Result<(), GuardReason>> + 'static,
    {
        self.prepare = Some(Rc::new(move || Box::pin(prepare())));

        self
    }
}

/// An explicit decision request returned by an application guard.
pub struct DecisionRequest {
    /// Human-readable prompt.
    pub message: String,
    /// Nonempty, unique, stable choices.
    pub choices: Vec<DecisionChoice>,
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

/// Native/embedding capability for opening and retiring physical windows.
///
/// Install before driving the router. `open` creates the host needed by the route
/// mount adapter; its future must clean up on failure/cancellation. After success,
/// Rake calls `close` if mounting is cancelled or fails. Hosts must not retain a
/// strong router in this adapter, which would form an ownership cycle. Browser
/// runtimes reject this capability and never turn NewWindow into NewTab.
pub trait WindowHost: 'static {
    /// Creates an empty host on the UI loop, honoring the requested activation.
    fn open(
        &self,
        id: WindowId,
        activation: Activation,
    ) -> LocalBoxFuture<'static, Result<(), MountError>>;

    /// Removes an already approved or provisional host. Must tolerate repeated cleanup.
    fn close(&self, id: WindowId);

    /// Brings an existing host forward after explicit selection/reveal.
    fn activate(&self, id: WindowId);
}

fn initial_window() -> WindowId {
    WindowId(1)
}
