//! Local managed work: scopes own jobs; entries own restartable activity factories.
//!
//! Work runs on the host event loop without a `Send` requirement. Async I/O yields;
//! this is not a worker-thread API. Cancellation invalidates publication immediately
//! and drops the future when the driver next runs; it does not undo external effects.
//!
//! ```
//! use rake::prelude::*;
//! use rake::tasks::TaskState;
//! let router = Router::<rake::router::NoRoute>::new(RouterConfig::default());
//! let task = router.app().tasks().spawn("export", async { Ok(()) }).unwrap();
//! assert_eq!(task.snapshot().unwrap().state, TaskState::Queued);
//! // The host drives router.run(); dropping the observation handle does not cancel.
//! ```

#[cfg(feature = "gpui")]
pub(crate) mod host;
mod runtime;

pub use runtime::{ActivityRun, AppHandle, TaskHandle, TaskScope};
pub(crate) use runtime::{Scope, WorkRuntime};

/// Portable host timer. Dropping its future cancels observation of the timer.
pub async fn sleep(duration: std::time::Duration) {
    crate::http::sleep(duration).await;
}

pub use rake_protocol::tasks::*;
