//! GPUI action for guarded application quit.
pub use rake_core::quit::*;

#[cfg(feature = "gpui")]
gpui::actions!(
    rake,
    [
        /// Requests guarded application quit with the default wait-for-jobs policy.
        RequestQuit,
    ]
);
