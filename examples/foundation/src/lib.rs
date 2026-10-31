//! One route-free application with automatically bound asynchronous loading.

use rake::gpui::{Context, Window, prelude::*};
use rake::prelude::*;
use rake::updates::UpdateState;

#[rake::data]
struct Message {
    title: String,
}

struct Foundation {
    message: Loader<Message, rake::http::Error>,
    updates: UpdateHandle,
    status: String,
    runtime: RuntimeContext,
    clipboard: Option<AsyncAction<(), rake::platform::Error>>,
    reload: Option<AsyncAction<(), rake::web::Error>>,
    deployment: Option<rake::web::Deployment>,
    web_updates: Option<Loader<Option<rake::web::DeploymentVersion>, rake::web::Error>>,
}

#[rake::instrument(skip_all)]
async fn load_message(load: LoadContext) -> rake::http::Result<Message> {
    load.http()
        .get("message.json")
        .cancellation(load.cancellation())
        .send_json()
        .await
}

impl Render for Foundation {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let loading = self.message.is_loading(cx);

        let message = match self.message.state(cx) {
            LoadStatus::Idle | LoadStatus::Loading(_) => "Loading…".into(),
            LoadStatus::Ready(message) => message.title.clone(),
            LoadStatus::Failed(error) => error.to_string(),
        };

        let version = self.updates.available_version(cx);
        let update_error = self.updates.error(cx);
        let can_check = self.updates.can_check(cx);
        let can_download = self.updates.can_download(cx);
        let can_restart = self.updates.can_restart(cx);

        let clipboard_status = self
            .clipboard
            .as_ref()
            .map(|action| match action.state(cx) {
                LoadStatus::Ready(()) => "Copied to clipboard".into(),
                LoadStatus::Failed(error) => error.to_string(),
                _ => "Copying…".into(),
            });

        let reload_error = self
            .reload
            .as_ref()
            .and_then(|action| match action.state(cx) {
                LoadStatus::Failed(error) => Some(error.to_string()),
                _ => None,
            });

        let update_status = match self.updates.state(cx) {
            UpdateHandleState::Unavailable => "Updates unavailable in this build",
            UpdateHandleState::Observed(state) => match state {
                UpdateState::Disabled => "Updates disabled in this build",
                UpdateState::ManagedExternally(_) => "Updates managed by your package manager",
                UpdateState::Checking => "Checking for updates…",
                UpdateState::Downloading { .. } => "Downloading update…",
                UpdateState::Extracting { .. } => "Preparing update…",
                UpdateState::Available(_) => "A new version is available",
                UpdateState::ReadyToRestart(_) => "Ready to restart",
                UpdateState::AwaitingApproval(_) => "Waiting for save/quit approval…",
                UpdateState::Restarting(_) => "Restarting…",
                UpdateState::UpToDate => "You're up to date",
                _ => "Updates",
            },
        };

        let web_version = self
            .web_updates
            .as_ref()
            .map(|updates| match updates.state(cx) {
                LoadStatus::Ready(Some(version)) => {
                    format!("Web version {} is available", version.version)
                }
                LoadStatus::Ready(None) => "Web deployment is current".into(),
                LoadStatus::Failed(error) => error.to_string(),
                _ => "Checking Web deployment…".into(),
            });

        let can_reload_web = self
            .web_updates
            .as_ref()
            .is_some_and(|updates| matches!(updates.state(cx), LoadStatus::Ready(Some(_))));

        rake::view! {
            <main class="size-full p-6 gap-4 bg-[#18232b] text-[#f0f4e8]" tab-navigation>
                <h1>"Rake"</h1>
                <p>{message}</p>
                <button id="load-message" disabled={loading}
                    on:click={cx.listener(|this, _, _, cx| this.message.retry(cx))}>
                    "Reload / Retry"
                </button>
                <button id="managed-job" on:click={cx.listener(|this, _, _, cx| {
                    let work = this.runtime.tasks().spawn("example-job", async {
                        rake::tasks::sleep(std::time::Duration::from_secs(1)).await;
                        rake::info!("Managed example job finished");
                        Ok(())
                    });
                    this.status = match work {
                        Ok(_) => "Job started; normal quit waits for it to finish".into(),
                        Err(error) => error.to_string(),
                    };
                    cx.notify();
                })}>"Run managed job"</button>
                <p>{self.status.clone()}</p>
                <button id="copy-message" disabled={!self.runtime.services().platform().capabilities().clipboard_write.supported}
                    on:click={cx.listener(|this, _, _, cx| {
                        let text = match this.message.state(cx) {
                            LoadStatus::Ready(message) => message.title.clone(),
                            _ => "Rake".into(),
                        };
                        let request = this.runtime.services().platform().write_clipboard(&text, cx);
                        this.clipboard = Some(this.runtime.action(cx, request));
                    })}>"Copy message"</button>
                if let Some(status) = clipboard_status { <p>{status}</p> }
                <p>{update_status}</p>
                if let Some(version) = version { <p>{format!("Version {version}")}</p> }
                <div class="flex flex-row gap-3">
                    <button disabled={!can_check}
                        on:click={cx.listener(|this, _, _, cx| this.updates.check(cx))}>
                        "Check for updates"
                    </button>
                    <button disabled={!can_download}
                        on:click={cx.listener(|this, _, _, cx| this.updates.download(cx))}>
                        "Download"
                    </button>
                    <button disabled={!can_restart}
                        on:click={cx.listener(|this, _, _, cx| this.updates.restart(cx))}>
                        "Restart to update"
                    </button>
                </div>
                if let Some(error) = update_error { <p>{error}</p> }
                if let Some(version) = web_version {
                    <p>{version}</p>
                    <button on:click={cx.listener(|this, _, _, cx| {
                        if let Some(updates) = &mut this.web_updates { updates.retry(cx); }
                    })}>"Check Web deployment"</button>
                    <button disabled={!can_reload_web} on:click={cx.listener(|this, _, _, cx| {
                        let Some(deployment) = this.deployment.clone() else { return; };
                        let runtime = this.runtime.clone();
                        this.reload = Some(this.runtime.action(cx, async move {
                            drop(runtime.reload_web_update(&deployment, QuitOptions::default()).await?);
                            Ok(())
                        }));
                    })}>"Reload to update"</button>
                    if let Some(error) = reload_error { <p>{error}</p> }
                }
            </main>
        }
    }
}

/// The same factory configures the native binary and browser Wasm export.
pub fn application() -> App {
    App::new("com.example.rake.foundation")
        .http(HttpConfig::new().base_url("http://127.0.0.1:8080/"))
        .root_with(|runtime, _, cx| {
            cx.new(|cx| {
                let (deployment, status) = match rake::web::Deployment::from_document() {
                    Ok(deployment) => (deployment, "Portable application foundation".into()),
                    Err(error) => (None, error.to_string()),
                };

                let web_updates = deployment.clone().map(|deployment| {
                    runtime.load(cx, move |_| {
                        let deployment = deployment.clone();

                        async move { deployment.check().await }
                    })
                });

                Foundation {
                    message: runtime.load(cx, load_message),
                    updates: runtime.bind_updates(cx),
                    status,
                    runtime,
                    clipboard: None,
                    reload: None,
                    deployment,
                    web_updates,
                }
            })
            .into()
        })
}

#[rake::app]
fn app() -> App {
    application()
}
