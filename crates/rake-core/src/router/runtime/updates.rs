//! Provider-neutral update actions share the router's existing guarded quit.
use super::{Operation, Route, Router};
use crate::quit::{QuitOptions, QuitResult};
use rake_updates::{Error, UpdateController};
use std::rc::Rc;

impl<R: Route> Router<R> {
    /// Returns the application's shared update model, when installed policy is available.
    pub fn updates(&self) -> Option<Rc<UpdateController>> {
        self.client().services().get().ok()
    }

    /// Checks the installed provider without downloading or requesting termination.
    /// # Errors
    /// Rejects unavailable or externally managed providers and concurrent operations.
    pub fn check_for_updates(&self) -> Result<(), Error> {
        self.updates()
            .ok_or_else(|| Error::Policy("no update controller is installed".into()))?
            .check()
    }

    /// Downloads the selected update; Sparkle uses its native review UI.
    /// # Errors
    /// Rejects missing availability or another active operation.
    pub fn download_update(&self) -> Result<(), Error> {
        self.updates()
            .ok_or_else(|| Error::Policy("no update controller is installed".into()))?
            .download()
    }

    /// Submits verified staging to the existing save, work and window quit guards.
    /// # Errors
    /// Rejects absent staging or a competing quit operation.
    pub fn restart_for_update(&self, options: QuitOptions) -> Result<Operation<QuitResult>, Error> {
        #[cfg(target_family = "wasm")]
        {
            let _ = options;

            Err(Error::Policy(
                "native update restart is unavailable in a browser".into(),
            ))
        }

        #[cfg(not(target_family = "wasm"))]
        {
            let updates = self
                .updates()
                .ok_or_else(|| Error::Policy("no update controller is installed".into()))?;

            let handoff = updates.restart_handoff(std::env::args_os().skip(1).collect())?;

            self.quit_for_update(handoff, options)
        }
    }
}
