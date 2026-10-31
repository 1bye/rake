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

pub use crate::router::QuitGuardContext;
pub use rake_protocol::quit::*;

#[cfg(feature = "gpui")]
gpui::actions!(
    rake,
    [
        /// Requests guarded application quit with the default wait-for-jobs policy.
        RequestQuit,
    ]
);
