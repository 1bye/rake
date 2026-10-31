//! Typed, side-effect-free admission callbacks for route transitions.

use std::rc::Rc;

use futures::future::LocalBoxFuture;

use super::{GuardReason, OperationId, Route, TabId, WindowId};
use crate::services::ServiceRegistry;

/// Result of evaluating an application-owned route admission policy.
///
/// `Redirect` is data returned to the router. It does not navigate as a side
/// effect and therefore remains part of the original operation's guard chain.
/// Admission callbacks run for destination-bearing `Open`, `Navigate`, `Reveal`,
/// and `Restore` operations. Retained-history cursor moves and tab selection
/// continue through entry guards because redirecting them would change their
/// cursor/browser history semantics.
pub enum Admission<R: Route> {
    /// Continue with the requested destination.
    Allow,
    /// Deliberately block the operation.
    Deny(GuardReason),
    /// Re-plan this operation for another typed destination.
    Redirect(R),
}

/// Owned context available before a destination is turned into a retained entry.
///
/// The context intentionally has no entry ID, revision, dirty state, or view
/// handle: admission runs before entry allocation, host creation, and mounting.
#[derive(Clone)]
pub struct AdmissionContext<R: Route> {
    /// Operation being admitted.
    pub operation_id: OperationId,
    /// Origin/presenting host. A provisional NewWindow has no host yet.
    pub window_id: WindowId,
    /// Concrete tab for CurrentTab/Navigate actions, when already resolved.
    pub target_tab: Option<TabId>,
    /// Candidate typed destination.
    pub destination: R,
    /// Immutable application service composition.
    pub services: ServiceRegistry,
}

pub(crate) type Callback<R> =
    Rc<dyn Fn(AdmissionContext<R>) -> LocalBoxFuture<'static, Admission<R>>>;
