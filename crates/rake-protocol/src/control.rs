//! Owned wire types. Serde rejects unknown fields and unknown command/query tags.

use crate::router::{
    DecisionError, DecisionId, OpenOptions, OpenTarget, OperationError, OperationId,
    OperationState, RevealScope, RouteAddress, RouteDecodeError, RouterSnapshot, RuntimeId, TabId,
    TabMode, WindowId,
};

use crate::tasks::{TaskError, TaskId, TaskOwner};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Current version of the development control protocol.
pub const PROTOCOL_VERSION: u32 = 1;
/// Maximum encoded request/response size admitted by development transports.
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

/// A request for one exact runtime; application mutations require a stable retry key.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Required exact protocol version.
    pub protocol: u32,
    /// Prevents stale IDs from accidentally targeting a restarted application.
    pub runtime_id: RuntimeId,
    /// Caller-generated key, 1–128 bytes; required except for transient inspection controls.
    pub key: Option<String>,
    /// Shared command or diagnostic query.
    pub message: Message,
}

/// Runtime mutation or read-only query.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Message {
    /// Application actions, decision responses or transient development inspection controls.
    Command(Command),
    /// Owned state/schema/diagnostic inspection.
    Query(Query),
}

/// Shared runtime commands; transient inspection controls never mutate router state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    /// Edits rendered text without changing application state or Rust source.
    EditElementText {
        /// Exact current text boundary.
        id: String,
        /// Exact current capture; stale requests never retarget.
        generation: String,
        /// Bounded literal replacement, reset or undo.
        edit: crate::inspection::TextEdit,
    },
    /// Enables a renewable frame recording lease; disconnect expiry stops measurement.
    RecordFrames {
        /// Start/renew recording, or stop without discarding retained frames.
        enabled: bool,
        /// Must be 1–5000 milliseconds when enabled.
        lease_ms: u64,
    },
    /// Edits temporary base refinements without changing Rust source or router state.
    EditElementStyle {
        /// Exact current boundary.
        id: String,
        /// Exact current capture.
        generation: String,
        /// Validated transient operation; no automatic delivery retry.
        edit: crate::inspection::StyleEdit,
    },
    /// Navigate to an exact presented boundary; no application operation is retained.
    NavigateElement {
        /// Exact capture element identity.
        id: String,
        /// Exact capture generation; stale requests never retarget.
        generation: String,
        /// Supported focus or scroll action.
        action: crate::inspection::NavigationAction,
    },
    /// Highlight a capture boundary for a two-second development lease; no operation is retained.
    HighlightElement {
        /// Exact capture ID, or None to clear all window highlights.
        id: Option<String>,
        /// Capture identity returned by InspectTree; required when setting a highlight.
        generation: String,
    },
    /// Temporarily intercept window input to pick an inspected boundary.
    InspectMode {
        /// Enable or clear the development picker.
        enabled: bool,
        /// Renewal lease in milliseconds, 1–2000 when enabled.
        lease_ms: u64,
    },
    /// Guard all retained entries and close exactly one native host.
    CloseWindow {
        /// Concrete runtime-scoped host.
        window: WindowId,
    },
    /// Guard every retained entry, coordinate jobs, then dispose the workspace.
    Quit {
        /// Explicit finite-job policy.
        #[serde(default)]
        options: crate::quit::QuitOptions,
    },
    /// Enter a quiescent development boundary.
    PrepareReload {
        /// Stable supervisor boundary identity and actual mechanism.
        request: crate::reload::ReloadRequest,
    },
    /// Commit supported native replacement or abort and resume the old runtime.
    FinishReload {
        /// Prepared boundary ID.
        id: String,
        /// External backend asserts successful supported code replacement.
        applied: bool,
    },
    /// Open a typed destination with explicit activation and policy.
    Open {
        /// Concrete source host; omission selects the runtime's initial host.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        window: Option<WindowId>,
        /// Shared route address.
        destination: RouteAddress,
        /// Creation/activation options.
        options: OpenOptions,
    },
    /// Push or replace within a concrete tab.
    Navigate {
        /// Exact tab.
        tab: TabId,
        /// Shared route address.
        destination: RouteAddress,
        /// Replace rather than push.
        replace: bool,
    },
    /// Select an exact matching current destination or create it.
    Reveal {
        /// Bound host used for lookup/creation.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        window: Option<WindowId>,
        /// Application lookup by default, or restrict matches to the bound host.
        #[serde(default, skip_serializing_if = "application_scope")]
        scope: RevealScope,
        /// Shared route address.
        destination: RouteAddress,
    },
    /// Select an independent tab.
    Select {
        /// Exact tab.
        tab: TabId,
    },
    /// Guard and dispose an independent tab.
    Close {
        /// Exact tab.
        tab: TabId,
    },
    /// Traverse this tab's retained history.
    Back {
        /// Exact tab.
        tab: TabId,
    },
    /// Traverse this tab's retained forward history.
    Forward {
        /// Exact tab.
        tab: TabId,
    },
    /// Change current-entry managed-work policy.
    Mode {
        /// Exact tab.
        tab: TabId,
        /// Explicit Foreground or Live policy.
        mode: TabMode,
    },
    /// Request cancellation before commit.
    CancelOperation {
        /// Operation in this runtime.
        operation: OperationId,
    },
    /// Resolve the same revision-checked decision shown in the GUI.
    Respond {
        /// Pending decision.
        decision: DecisionId,
        /// Stable choice ID.
        choice: String,
    },
    /// Run an explicitly registered typed application operation.
    Run {
        /// Stable registered key.
        id: String,
        /// Typed arguments decoded by the registry.
        arguments: Value,
    },
    /// Cancel a task and unregister any activity factory.
    CancelTask {
        /// Retained task identity.
        task: TaskId,
    },
}

impl Command {
    /// Stable tags accepted by Serde, used in discovery and terminal help.
    #[must_use]
    pub fn names() -> &'static [&'static str] {
        &[
            "record_frames",
            "edit_element_style",
            "edit_element_text",
            "navigate_element",
            "highlight_element",
            "inspect_mode",
            "close_window",
            "quit",
            "prepare_reload",
            "finish_reload",
            "open",
            "navigate",
            "reveal",
            "select",
            "close",
            "back",
            "forward",
            "mode",
            "cancel_operation",
            "respond",
            "cancel_task",
            "run",
        ]
    }
}

/// Shared diagnostic queries; inspection replies report actual host availability.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Query {
    /// Retained frame timings after a recording sequence, with loss and continuation metadata.
    Frames {
        /// Exclusive recording cursor, zero starts at retained history.
        after: u64,
    },
    /// Leased picker status; clients deduplicate retained clicks by pick_sequence.
    InspectPick {},
    /// All logical host identities, independent selections, visibility and close status.
    Windows {},
    /// Routing snapshot restricted to one concrete host.
    Window {
        /// Runtime-scoped host.
        window: WindowId,
    },
    /// Quit phase and finite work still preventing shutdown.
    Quit {},
    /// Latest bounded presented tree, with explicit availability and coverage.
    InspectTree {},
    /// Lightweight generation-pinned tree index, at most 256 boundaries per response.
    InspectTreePage {
        /// None starts at the current capture; subsequent pages must pin it.
        generation: Option<String>,
        /// Zero-based index into the retained depth-first tree.
        offset: usize,
    },
    /// Exact boundary in the latest completed capture; IDs never select by semantics.
    InspectElement {
        /// Opaque ID returned by InspectTree.
        id: String,
    },
    /// Reload boundary status, including quiescence and compatibility failures.
    Reload {},
    /// Capture logical state after readiness.
    Checkpoint {
        /// Expected application identity, retained in the checkpoint.
        application_id: String,
    },
    /// Version and actual capabilities.
    Info {},
    /// Protocol tags, envelope example and route parameter schemas.
    Schema {},
    /// Registered application command metadata.
    Commands {},
    /// Stable destination schemas.
    Routes {},
    /// Authoritative application state.
    Snapshot {},
    /// Independent tabs and histories.
    Tabs {},
    /// Tasks, optionally filtered by lifetime owner.
    Tasks {
        /// Lifetime owner, independent of causal origin.
        owner: Option<TaskOwner>,
    },
    /// Retained operation states.
    Operations {},
    /// One retained state and committed result.
    Operation {
        /// Runtime-scoped operation.
        operation: OperationId,
    },
    /// Pending structured decisions.
    Decisions {},
    /// Poll state events; cursor gaps require the supplied fresh snapshot.
    Events {
        /// Last consumed cursor, or None for an initial snapshot.
        cursor: Option<EventCursor>,
    },
    /// Bounded records for logs, requests and traces with observable loss counts.
    Diagnostics {
        /// Exclusive collector sequence.
        after: u64,
        /// Optional record kind such as `request`; `span` includes opens and closes.
        kind: Option<String>,
    },
}

impl Query {
    /// Stable tags accepted by Serde, used in discovery and terminal help.
    #[must_use]
    pub fn names() -> &'static [&'static str] {
        &[
            "windows",
            "window",
            "quit",
            "inspect_pick",
            "inspect_tree",
            "inspect_tree_page",
            "frames",
            "inspect_element",
            "reload",
            "checkpoint",
            "info",
            "schema",
            "routes",
            "snapshot",
            "tabs",
            "tasks",
            "operations",
            "operation",
            "decisions",
            "events",
            "diagnostics",
            "commands",
        ]
    }
}

/// Actual host capability advertisement; browser instances never advertise native IPC.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeInfo {
    /// Mounted inspection coverage availability; headless runtimes report Unavailable.
    #[serde(default)]
    pub element_inspection: crate::inspection::Status,
    /// Development highlight/picker support; false for disabled or headless hosts.
    #[serde(default)]
    pub element_interaction: bool,
    /// Protocol version.
    pub protocol: u32,
    /// This live application lifetime.
    pub runtime_id: RuntimeId,
    /// `native` or `web`.
    pub platform: String,
    /// Available mutation tags.
    pub commands: Vec<String>,
    /// Available query tags.
    pub queries: Vec<String>,
    /// Supported application host targets.
    pub supported_targets: Vec<OpenTarget>,
    /// Number of completed operation results retained; all pending states are retained.
    pub completed_operations: usize,
    /// Pending operation admission limit.
    pub pending_operations: usize,
    /// Lifetime retry-key admission limit; keys are never silently evicted.
    pub idempotency_capacity: usize,
}

/// Retained outcome independent of connection and Rust observation-handle lifetime.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OperationRecord {
    /// Runtime-scoped identity.
    pub operation_id: OperationId,
    /// Authoritative operation status.
    pub state: OperationState,
    /// Committed result IDs; None until commit or for non-committed outcomes.
    pub result: Option<Value>,
}

/// Runtime/epoch-aware cursor. A full host restart changes runtime identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventCursor {
    /// Runtime identity.
    pub runtime_id: RuntimeId,
    /// Protocol state-stream epoch, currently 1.
    pub epoch: u64,
    /// Authoritative application revision.
    pub revision: u64,
}

/// State stream outcome. This adapter retains snapshots, not intermediate mutations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// Initial authoritative snapshot.
    Snapshot,
    /// Exactly one subsequent revision observed.
    Changed,
    /// No new revision.
    Unchanged,
    /// A skipped revision, stale epoch or different runtime requires resynchronization.
    Gap,
}

/// State notification with a complete snapshot whenever the revision changes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeEvent {
    /// Cursor for the enclosed snapshot.
    pub cursor: EventCursor,
    /// Explicit consistency outcome.
    pub kind: EventKind,
    /// Full state except for Unchanged events.
    pub snapshot: Option<RouterSnapshot>,
}

/// Shared response format for local clients, CLI, native IPC and browser broker.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Reply {
    /// Capture generation, hover and sequenced click selection.
    InspectPick(crate::inspection::Pick),
    /// Guarded application shutdown status.
    Quit(crate::quit::QuitStatus),
    /// Latest completed bounded element tree.
    Elements(crate::inspection::Snapshot),
    /// Bounded live frame timings.
    Frames(crate::inspection::Frames),
    /// Generation-pinned tree index page.
    ElementPage(crate::inspection::TreePage),
    /// Exact element metadata from that capture.
    Element(crate::inspection::Element),
    /// Observable reload phase.
    Reload(crate::reload::ReloadStatus),
    /// Owned development workspace.
    Checkpoint(crate::reload::WorkspaceCheckpoint),
    /// Protocol/host discovery.
    Info(RuntimeInfo),
    /// Owned route/tab/task/decision/schema data.
    Data(Value),
    /// Complete authoritative application state.
    Snapshot(RouterSnapshot),
    /// A retained operation and its committed result.
    Operation(OperationRecord),
    /// Eager operation submission, not proof of commit.
    Submitted {
        /// Queryable operation identity.
        operation_id: OperationId,
    },
    /// Cancellation admission; false means it is already too late.
    Accepted {
        /// Whether cancellation was admitted.
        accepted: bool,
    },
    /// A decision response was admitted and will be revalidated.
    Acknowledged,
    /// Revision-aware state notification.
    Event(RuntimeEvent),
    /// Bounded diagnostic records, including loss counts.
    Diagnostics(rake_diagnostics::Snapshot),
}

/// Structured protocol errors; transport failures are reported separately as I/O errors.
#[derive(Clone, Debug, thiserror::Error, Serialize, Deserialize)]
pub enum ControlError {
    /// This host does not implement the requested operation; consult RuntimeInfo capabilities.
    #[error("this host does not support {operation}")]
    Unsupported {
        /// Unsupported command/query category.
        operation: String,
    },
    /// Inspection availability or exact capture lookup failure.
    #[error(transparent)]
    Inspection(#[from] crate::inspection::InspectionError),
    /// Development preparation or compatibility failure.
    #[error(transparent)]
    Reload(#[from] crate::reload::ReloadError),
    /// Unsupported protocol version.
    #[error("unsupported control protocol; expected {expected}")]
    ProtocolVersion {
        /// Required version.
        expected: u32,
    },
    /// Stale or incorrect instance selector.
    #[error("the request targets another runtime")]
    WrongRuntime,
    /// Host teardown stopped admission.
    #[error("the runtime has shut down")]
    RuntimeShutdown,
    /// Missing or invalid mutation idempotency key.
    #[error("commands require a request key of 1–128 bytes")]
    InvalidRequestKey,
    /// A retry changed its command payload.
    #[error("the request key was already used for a different command")]
    KeyConflict,
    /// Lifetime key registry full. Existing keys remain retryable.
    #[error("runtime request-key capacity reached")]
    RequestCapacity,
    /// Application registry, arguments or capability validation.
    #[error(transparent)]
    Command(#[from] CommandError),
    /// Shared destination validation.
    #[error(transparent)]
    Route(#[from] RouteDecodeError),
    /// Routing admission, retention or technical error.
    #[error(transparent)]
    Operation(#[from] OperationError),
    /// Revision-sensitive guard choice error.
    #[error(transparent)]
    Decision(#[from] DecisionError),
    /// Managed-work lookup or cancellation error.
    #[error(transparent)]
    Task(#[from] TaskError),
}

fn application_scope(scope: &RevealScope) -> bool {
    *scope == RevealScope::Application
}

/// Command registration, typed argument, capability or handler failure.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum CommandError {
    /// The stable command key was never registered.
    #[error("unknown application command: {0}")]
    Unknown(String),
    /// Stable key invalid or duplicated.
    #[error("invalid or duplicate application command: {0}")]
    Registration(String),
    /// Typed argument decoding failed.
    #[error("invalid command arguments: {0}")]
    Arguments(String),
    /// A UI-dependent handler cannot execute headlessly.
    #[error("this command requires a window")]
    RequiresWindow,
    /// The handler deliberately reported an application failure.
    #[error("application command failed: {0}")]
    Failed(String),
    /// Typed result could not be serialized.
    #[error("command result serialization failed: {0}")]
    Result(String),
}
