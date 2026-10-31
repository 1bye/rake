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

/// What the built-in host adapter can measure, independently of keyboard focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisibilitySupport {
    /// No portable measurement; custom hosts may supply their own signal.
    Unknown,
    /// AppKit visibility, minimization and occlusion.
    NativeOcclusion,
    /// Windows visibility, minimization and DWM cloaking; overlap is not measured.
    NativeWindowState,
    /// Browser document visibility/page lifecycle; execution may still be suspended.
    BrowserDocument,
}

/// Describes the selected adapter's visibility contract. A particular unavailable
/// surface can still report [`HostVisibility::Unknown`]. Linux/Wayland do not infer
/// visibility from focus; embedding hosts can use `Router::set_window_visibility`.
pub fn visibility_support() -> VisibilitySupport {
    if !cfg!(feature = "gpui") {
        return VisibilitySupport::Unknown;
    }
    if cfg!(target_os = "macos") {
        VisibilitySupport::NativeOcclusion
    } else if cfg!(target_os = "windows") {
        VisibilitySupport::NativeWindowState
    } else if cfg!(target_family = "wasm") {
        VisibilitySupport::BrowserDocument
    } else {
        VisibilitySupport::Unknown
    }
}

/// Portable host timer. Dropping its future cancels observation of the timer.
pub async fn sleep(duration: std::time::Duration) {
    crate::http::sleep(duration).await;
}

pub use rake_protocol::tasks::*;
