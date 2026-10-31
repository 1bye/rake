//! One shared controller, with a visible observer owned by each retained page.
use super::Destination;
use rake::gpui::{self, Context, Entity, Window, prelude::*};
use rake::prelude::*;
use rake::router::MountError;
use rake::updates::{Error, UpdateController, UpdateSnapshot, UpdateState};
use std::rc::Rc;

#[derive(Clone)]
struct UpdateRevision(UpdateSnapshot);

pub(crate) struct Updates {
    page: PageContext<Destination>,
    controller: Option<Rc<UpdateController>>,
    error: Option<String>,
    _observe: Option<TaskHandle>,
}

impl Page<Destination> for Updates {
    fn mount(
        _: &Destination,
        page: PageContext<Destination>,
        _: &mut Window,
        cx: &mut gpui::App,
    ) -> Result<Entity<Self>, MountError> {
        let controller = page.services().get::<UpdateController>().ok();
        let view = cx.new(|_| Self::new(page.clone(), controller.clone()));
        let weak = view.downgrade();
        let context = cx.to_async();

        page.on_event(move |UpdateRevision(_snapshot)| {
            weak.update(&mut context.clone(), |_, cx| cx.notify())
                .map_err(|error| TaskError::Failed(error.to_string()))
        })
        .map_err(|error| MountError(error.to_string()))?;

        let observation = controller
            .clone()
            .map(|updates| {
                page.while_visible("updates.observe", move |run| {
                    let updates = updates.clone();
                    async move {
                        let mut snapshot = updates.snapshot();
                        run.publish(UpdateRevision(snapshot.clone()))?;

                        loop {
                            snapshot = updates.changed(snapshot.revision).await;
                            run.publish(UpdateRevision(snapshot.clone()))?;
                        }
                    }
                })
                .map_err(|error| MountError(error.to_string()))
            })
            .transpose()?;

        view.update(cx, |view, _| view._observe = observation);
        Ok(view)
    }
}

impl Updates {
    fn new(page: PageContext<Destination>, controller: Option<Rc<UpdateController>>) -> Self {
        Self {
            page,
            controller,
            error: None,
            _observe: None,
        }
    }

    fn request(
        &mut self,
        action: fn(&UpdateController) -> Result<(), Error>,
        cx: &mut Context<Self>,
    ) {
        if let Some(updates) = &self.controller {
            self.error = action(updates).err().map(|error| error.to_string());
        }
        cx.notify();
    }
}

impl Render for Updates {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.controller.as_ref().map(|updates| updates.state());
        let label = match &state {
            Some(UpdateState::Idle) => "Check for a new version".into(),
            Some(UpdateState::Checking) => "Checking…".into(),
            Some(UpdateState::Available(info)) => format!("Version {} is available", info.version),
            Some(UpdateState::Downloading { progress, .. }) => {
                format!(
                    "Downloading: {} / {} bytes",
                    progress.received_bytes, progress.total_bytes
                )
            }
            Some(UpdateState::Extracting { .. }) => "Preparing update…".into(),
            Some(UpdateState::ReadyToRestart(info)) => {
                format!("Version {} is ready to install", info.version)
            }
            Some(UpdateState::AwaitingApproval(_)) => "Waiting for save/quit approval…".into(),
            Some(UpdateState::Restarting(_)) => "Restarting…".into(),
            Some(UpdateState::Cancelling) => "Cancelling…".into(),
            Some(UpdateState::UpToDate) => "You're up to date".into(),
            Some(UpdateState::Failed { message }) => message.clone(),
            Some(UpdateState::ManagedExternally(_)) => {
                "Updates managed by your package manager".into()
            }
            Some(UpdateState::Disabled) => "Updates are disabled in this build".into(),
            _ => "Updates are unavailable in this build".into(),
        };
        let can_check = matches!(
            state,
            Some(UpdateState::Idle | UpdateState::UpToDate | UpdateState::Failed { .. })
        );
        let can_download = matches!(state, Some(UpdateState::Available(_)))
            && self
                .controller
                .as_ref()
                .is_some_and(|updates| updates.can_install());
        let can_restart = matches!(state, Some(UpdateState::ReadyToRestart(_)));
        let can_cancel = matches!(
            state,
            Some(UpdateState::Checking | UpdateState::Downloading { .. })
        );

        rake::view! {
            <section class="gap-3">
                <h2>"Updates"</h2>
                <p>{label}</p>
                <div class="flex flex-row gap-3">
                    <button disabled={!can_check} class="disabled:opacity-50"
                        on:click={cx.listener(|this, _, _, cx| {
                            this.request(UpdateController::check, cx);
                        })}>"Check for updates"</button>
                    <button disabled={!can_download} class="disabled:opacity-50"
                        on:click={cx.listener(|this, _, _, cx| {
                            this.request(UpdateController::download, cx);
                        })}>"Download"</button>
                    <button disabled={!can_cancel} class="disabled:opacity-50"
                        on:click={cx.listener(|this, _, _, cx| {
                            if let Some(updates) = &this.controller { updates.cancel(); }
                            this.error = None;
                            cx.notify();
                        })}>"Cancel"</button>
                    <button disabled={!can_restart} class="disabled:opacity-50"
                        on:click={cx.listener(|this, _, _, cx| {
                            this.error = this.page.runtime().restart_for_update(QuitOptions::default())
                                .err().map(|error| error.to_string());
                            cx.notify();
                        })}>"Restart to update"</button>
                </div>
                if let Some(error) = &self.error {
                    <p>{error.clone()}</p>
                }
            </section>
        }
    }
}
