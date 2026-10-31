//! Runtime-owned handles, guard callbacks and configuration.

use std::any::Any;

use super::{
    Activation, ChoiceAction, ChoiceDescriptor, EntryId, GuardKind, GuardReason, MountError,
    OperationError, OperationId, Outcome, Route, TabId, TabMode, WindowId,
};

/// Opaque, owned data prepared before a route is mounted.
///
/// This is an internal bridge between the statically dispatched route loader
/// and the runtime's erased mount callback. The value is consumed exactly once;
/// dropping it drops the loaded data as well. Ordinary applications should use
/// the `loaded(...)` arm of `rake::routes!` rather than this type directly.
#[doc(hidden)]
pub struct PreparedPage(Option<Box<dyn Any>>);

impl PreparedPage {
    /// Creates an empty preparation for synchronous route views.
    #[doc(hidden)]
    pub fn empty() -> Self {
        Self(None)
    }

    /// Wraps owned route data for a custom preparation implementation.
    #[doc(hidden)]
    pub fn from_value<T: 'static>(value: T) -> Self {
        Self(Some(Box::new(value)))
    }

    /// Returns whether this preparation carries no route data.
    #[doc(hidden)]
    pub fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    /// Consumes the preparation and recovers its concrete route data.
    #[doc(hidden)]
    pub fn downcast<T: 'static>(self) -> Result<T, MountError> {
        self.0
            .ok_or_else(|| MountError("route preparation is empty".into()))?
            .downcast::<T>()
            .map(|value| *value)
            .map_err(|_| {
                MountError(format!(
                    "route preparation type mismatch for {}",
                    std::any::type_name::<T>()
                ))
            })
    }
}

use crate::quit::QuitOptions;

use futures::future::{LocalBoxFuture, Shared};
use std::rc::Rc;

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
    /// Maximum typed admission redirects permitted in one operation.
    pub max_admission_redirects: usize,
    /// Number of terminal navigation statuses retained independently of operations.
    /// The effective minimum is one.
    /// Pending statuses are never trimmed; one recoverable status is retained per
    /// currently empty live window even when this bound is reached.
    pub navigation_records: usize,
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
            max_admission_redirects: 8,
            navigation_records: 64,
            default_mode: TabMode::Foreground,
        }
    }
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

/// Application-wide quit guard input.
///
/// Rake invokes these guards after finite work and retained-entry guards have
/// approved the quit, immediately before managed work is stopped. The callback
/// owns any application state it needs to inspect and should re-read that state
/// before returning `Ok(())`; Rake cannot revalidate state outside the router.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuitGuardContext {
    /// The quit operation being evaluated.
    pub operation_id: OperationId,
    /// The finite-work policy selected by the initiating quit request.
    pub options: QuitOptions,
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
