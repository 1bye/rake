//! Native quit bridge for the GPUI host.

use super::{Route, Router};
use crate::quit::{QuitOptions, QuitPhase};
use futures::FutureExt;
#[cfg(not(feature = "updates"))]
use futures::future::select;
use std::rc::Rc;

impl<R: Route> Router<R> {
    pub(crate) fn install_native_quit(&self) {
        let Some(host) = self.gpui_host() else {
            return;
        };

        if host.native_quit.borrow().is_none() {
            match rake_updates::NativeQuitBridge::new() {
                Ok(bridge) => *host.native_quit.borrow_mut() = Some(Rc::new(bridge)),
                Err(error) => tracing::error!(
                    error_type = std::any::type_name_of_val(&error),
                    "native quit guard could not be installed"
                ),
            }
        }
    }

    pub(crate) fn native_quit_changed(&self) -> futures::future::LocalBoxFuture<'static, ()> {
        let changed = self.native_quit_bridge().map(|bridge| bridge.changed());

        async move {
            if let Some(changed) = changed {
                changed.await;
            } else {
                std::future::pending::<()>().await;
            }
        }
        .boxed_local()
    }

    #[cfg(not(feature = "updates"))]
    pub(crate) async fn changed_with_native_quit(&self, revision: u64) -> u64 {
        let changed = self.changed(revision);
        let _ = select(Box::pin(changed), self.native_quit_changed()).await;

        self.poll_native_quit();

        self.revision()
    }

    pub(crate) fn poll_native_quit(&self) {
        let requested = self
            .native_quit_bridge()
            .is_some_and(|bridge| bridge.take_request());

        if requested {
            drop(self.quit(QuitOptions::default()));
        }

        if matches!(
            self.quit_status().phase,
            QuitPhase::Cancelled | QuitPhase::Blocked
        ) {
            self.reply_native_quit(false);
        }
    }

    pub(crate) fn prepare_native_exit(&self, approved: bool) {
        if let Some(bridge) = self.native_quit_bridge() {
            bridge.prepare_exit(approved);
        }
    }

    pub(crate) fn reply_native_quit(&self, approved: bool) {
        if let Some(bridge) = self.native_quit_bridge() {
            bridge.reply(approved);
        }
    }

    pub(super) fn native_quit_bridge(&self) -> Option<Rc<rake_updates::NativeQuitBridge>> {
        self.gpui_host()?.native_quit.borrow().clone()
    }
}
