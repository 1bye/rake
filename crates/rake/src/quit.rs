//! Guarded application shutdown, distinct from forced host termination.
//!
//! `Router::quit`/`Navigator::quit` return an eager shared operation. All retained
//! entries must approve disposal; finite jobs follow an explicit wait/cancel policy.
//! Before `Stopping`, cancellation preserves the workspace. `Ready` means managed
//! futures stopped and entries were disposed; the host owns process/page lifetime.
//!
//! ```
//! use rake::prelude::*;
//! use rake::router::NoRoute;
//! let router = Router::<NoRoute>::new(RouterConfig::default());
//! let quit = router.navigator().quit(QuitOptions::default());
//! assert_eq!(router.quit_status().operation_id, Some(quit.id()));
//! // Keep driving router.run() in the host; dropping this handle is observation only.
//! drop(quit);
//! ```

use crate::router::{OperationId, RuntimeId, TaskId};
use serde::{Deserialize, Serialize};

/// Policy for finite tasks and pending operations. Activity factories stop at commit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuitJobs {
    /// Wait for one-shot tasks and operations; callers can cancel quit or individual jobs.
    /// Observe quit outside those finite jobs, so a job does not wait on its own completion.
    #[default]
    Wait,
    /// Explicitly cancel pending operations and, after guards approve, one-shot tasks.
    /// Cancellation does not undo external effects, even if quit is later cancelled.
    Cancel,
}

/// Owned policy shared by GUI and control requests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuitOptions {
    /// Finite work policy; default Wait never silently cancels exports/downloads.
    #[serde(default)]
    pub jobs: QuitJobs,
}

/// Application quit lifecycle. Stopping is the irreversible cancellation boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuitPhase {
    /// No quit submitted.
    #[default]
    Idle,
    /// Existing operations or finite work must settle.
    WaitingForJobs,
    /// Checking all retained entries, including hidden histories.
    Guarding,
    /// Approval committed; cancelling activities and dropping managed futures.
    Stopping,
    /// Workspace disposed and managed work stopped. Native host may now exit.
    Ready,
    /// Quit cancelled; application admission remains open.
    Cancelled,
    /// A guard or technical failure prevented quit.
    Blocked,
}

impl QuitPhase {
    /// Whether an admitted quit is still executing.
    pub fn is_pending(self) -> bool {
        matches!(self, Self::WaitingForJobs | Self::Guarding | Self::Stopping)
    }

    pub(crate) fn blocks_navigation(self) -> bool {
        self.is_pending() || self == Self::Ready
    }
}

/// Authoritative quit state, separate from best-effort diagnostics.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QuitStatus {
    /// Current or most recent quit phase.
    pub phase: QuitPhase,
    /// Shared operation identity for the current/most recent request.
    pub operation_id: Option<OperationId>,
    /// Policy selected by the initiating request.
    pub options: QuitOptions,
    /// Active finite tasks, independent of the selected tab.
    pub pending_tasks: Vec<TaskId>,
    /// Pending routing/application operations, excluding the quit itself.
    pub pending_operations: Vec<OperationId>,
}

/// Committed quit: the workspace is disposed, not a promise that the OS exited.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuitResult {
    /// Runtime that completed its guarded shutdown.
    pub runtime_id: RuntimeId,
}

#[cfg(feature = "gpui")]
gpui::actions!(
    rake,
    [
        /// Requests guarded application quit with the default wait-for-jobs policy.
        RequestQuit,
    ]
);
