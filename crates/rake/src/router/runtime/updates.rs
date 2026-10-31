//! Provider-neutral update actions share the router's existing guarded quit.
use super::{Operation, Route, Router};
use crate::quit::{QuitOptions, QuitResult};
#[cfg(all(feature = "gpui", feature = "updates-sparkle", target_os = "macos"))]
use rake_updates::UpdateState;
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

#[cfg(all(feature = "gpui", not(target_family = "wasm")))]
impl<R: Route> Router<R> {
    pub(crate) async fn changed_with_updates(&self, revision: u64) -> u64 {
        let updates = self.updates();
        let update_revision = updates
            .as_ref()
            .map(|controller| controller.snapshot().revision);
        if let Some(next) = update_revision
            && self.0.update_revision.replace(next) != next
        {
            self.poll_updates();
            self.0.notify();
            return self.revision();
        }
        let changed = self.changed(revision);
        let update = async {
            if let (Some(updates), Some(revision)) = (&updates, update_revision) {
                updates.changed(revision).await;
            } else {
                std::future::pending::<()>().await;
            }
        };
        let changed = futures::future::select(Box::pin(changed), Box::pin(update));
        #[cfg(all(feature = "native-lifecycle", target_os = "macos"))]
        let _ = futures::future::select(Box::pin(changed), self.native_quit_changed()).await;
        #[cfg(not(all(feature = "native-lifecycle", target_os = "macos")))]
        let _ = changed.await;
        self.poll_updates();
        #[cfg(all(feature = "native-lifecycle", target_os = "macos"))]
        self.poll_native_quit();
        if let Some(updates) = &updates {
            let next = updates.snapshot().revision;
            if self.0.update_revision.replace(next) != next {
                self.0.notify();
            }
        }
        self.revision()
    }
    fn poll_updates(&self) {
        #[cfg(all(feature = "updates-sparkle", target_os = "macos"))]
        if let Some(updates) = self.updates() {
            if updates.binding().owner == rake_updates::UpdateOwner::Sparkle
                && matches!(updates.state(), UpdateState::ReadyToRestart(_))
                && let Err(error) = self.restart_for_update(QuitOptions::default())
            {
                tracing::error!(%error, "native update restart was not admitted");
            }
            if self.0.update_exit.get() == rake_updates::RestartDisposition::ProviderHandlesExit
                && let Some(status) = updates.native_status()
            {
                match status.stage {
                    rake_updates::SparkleStage::Installing => self
                        .0
                        .update_exit
                        .set(rake_updates::RestartDisposition::ExitHost),
                    rake_updates::SparkleStage::Failed => {
                        *self.0.update_restart_error.borrow_mut() = Some(
                            status
                                .error
                                .unwrap_or_else(|| "native installer failed".into()),
                        );
                        self.0.notify();
                    }
                    _ => {}
                }
            }
        }
    }
    #[cfg(all(feature = "updates-sparkle", target_os = "macos"))]
    pub(crate) fn install_sparkle(&self) -> Result<(), Error> {
        let Some(updates) = self.updates() else {
            return Ok(());
        };
        if updates.binding().owner != rake_updates::UpdateOwner::Sparkle {
            return Ok(());
        }
        let quit = self
            .0
            .native_quit
            .borrow()
            .as_ref()
            .cloned()
            .ok_or_else(|| Error::Policy("Sparkle requires an active native quit bridge".into()))?;
        updates.attach_sparkle(quit)
    }
}
