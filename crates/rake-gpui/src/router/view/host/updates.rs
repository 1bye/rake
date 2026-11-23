//! Provider and native-quit wakeups for the managed GPUI driver.

use super::{Route, Router};
#[cfg(all(feature = "updates-sparkle", target_os = "macos"))]
use crate::quit::QuitOptions;
#[cfg(all(feature = "updates-sparkle", target_os = "macos"))]
use rake_updates::{Error, UpdateState};

impl<R: Route> Router<R> {
    pub(crate) async fn changed_with_updates(&self, revision: u64) -> u64 {
        let Some(host) = self.gpui_host() else {
            return self.changed(revision).await;
        };

        let updates = self.updates();

        let update_revision = updates
            .as_ref()
            .map(|controller| controller.snapshot().revision);

        if let Some(next) = update_revision
            && host.update_revision.replace(Some(next)) != Some(next)
        {
            self.poll_updates();
            self.notify_update_changed();

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

            if host.update_revision.replace(Some(next)) != Some(next) {
                self.notify_update_changed();
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
                tracing::error!(
                    error_type = std::any::type_name_of_val(&error),
                    "native update restart was not admitted"
                );
            }

            if self.awaiting_provider_exit()
                && let Some(status) = updates.native_status()
            {
                match status.stage {
                    rake_updates::SparkleStage::Installing => self.update_provider_exiting(),
                    rake_updates::SparkleStage::Failed => self.report_update_restart_error(
                        status
                            .error
                            .unwrap_or_else(|| "native installer failed".into()),
                    ),
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
            .native_quit_bridge()
            .ok_or_else(|| Error::Policy("Sparkle requires an active native quit bridge".into()))?;

        updates.attach_sparkle(quit)
    }
}
