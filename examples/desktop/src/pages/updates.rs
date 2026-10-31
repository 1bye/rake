//! The view binds to the application's shared controller without a watcher.
use super::Destination;
use rake::gpui::{self, Context, Entity, Window, prelude::*};
use rake::prelude::*;
use rake::router::MountError;
use rake::updates::UpdateState;

pub(crate) struct Updates {
    updates: UpdateHandle,
}

impl Page<Destination> for Updates {
    fn mount(
        _: &Destination,
        page: PageContext<Destination>,
        _: &mut Window,
        cx: &mut gpui::App,
    ) -> Result<Entity<Self>, MountError> {
        Ok(cx.new(|cx| Self {
            updates: page.bind_updates(cx),
        }))
    }
}

impl Render for Updates {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let label = match self.updates.state(cx) {
            UpdateHandleState::Unavailable => "Updates are unavailable in this build".into(),
            UpdateHandleState::Observed(state) => match state {
                UpdateState::Idle => "Check for a new version".into(),
                UpdateState::Checking => "Checking…".into(),
                UpdateState::Available(info) => format!("Version {} is available", info.version),
                UpdateState::Downloading { progress, .. } => format!(
                    "Downloading: {} / {} bytes",
                    progress.received_bytes, progress.total_bytes
                ),
                UpdateState::Extracting { .. } => "Preparing update…".into(),
                UpdateState::ReadyToRestart(info) => {
                    format!("Version {} is ready to install", info.version)
                }
                UpdateState::AwaitingApproval(_) => "Waiting for save/quit approval…".into(),
                UpdateState::Restarting(_) => "Restarting…".into(),
                UpdateState::Cancelling => "Cancelling…".into(),
                UpdateState::UpToDate => "You're up to date".into(),
                UpdateState::Failed { .. } => "The update operation failed".into(),
                UpdateState::ManagedExternally(_) => {
                    "Updates managed by your package manager".into()
                }
                UpdateState::Disabled => "Updates are disabled in this build".into(),
                _ => "Updates are unavailable in this build".into(),
            },
        };
        let error = self.updates.error(cx);
        let can_check = self.updates.can_check(cx);
        let can_download = self.updates.can_download(cx);
        let can_restart = self.updates.can_restart(cx);
        let can_cancel = self.updates.can_cancel(cx);

        rake::view! {
            <section class="gap-3">
                <h2>"Updates"</h2>
                <p>{label}</p>
                <div class="flex flex-row gap-3">
                    <button disabled={!can_check} class="disabled:opacity-50"
                        on:click={cx.listener(|this, _, _, cx| this.updates.check(cx))}>
                        "Check for updates"
                    </button>
                    <button disabled={!can_download} class="disabled:opacity-50"
                        on:click={cx.listener(|this, _, _, cx| this.updates.download(cx))}>
                        "Download"
                    </button>
                    <button disabled={!can_cancel} class="disabled:opacity-50"
                        on:click={cx.listener(|this, _, _, cx| this.updates.cancel(cx))}>
                        "Cancel"
                    </button>
                    <button disabled={!can_restart} class="disabled:opacity-50"
                        on:click={cx.listener(|this, _, _, cx| this.updates.restart(cx))}>
                        "Restart to update"
                    </button>
                </div>
                if let Some(error) = error { <p>{error}</p> }
            </section>
        }
    }
}
