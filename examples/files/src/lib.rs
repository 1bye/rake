//! Route-free file utility exercising shared platform services and managed actions.

mod model;

use model::{Document, Error, Processed, Statistics, Workspace};
use rake::gpui::{self, Context, Window, prelude::*};
use rake::platform::{Preferences, SaveOutcome};
use rake::prelude::*;
use std::rc::Rc;

#[derive(Clone, rake::Theme)]
struct Colors {
    surface: gpui::Rgba,
    ink: gpui::Rgba,
}

rake::theme!(Colors);

#[rake::data(schema)]
#[derive(rake::Command)]
#[command(id = "document.statistics", output = Option<Statistics>, capability = Headless)]
struct InspectStatistics;

fn commands() -> Result<Commands, CommandError> {
    let commands = Commands::new();

    commands.register::<InspectStatistics, _, _>(|_, context| async move {
        let workspace = context
            .services
            .get::<Workspace>()
            .map_err(|error| CommandError::Failed(error.to_string()))?;

        Ok(workspace.current().map(|document| document.statistics))
    })?;

    Ok(commands)
}

async fn prepare(
    text: String,
    normalize: bool,
    trim: bool,
    executor: gpui::BackgroundExecutor,
) -> Result<Processed, Error> {
    #[cfg(not(target_family = "wasm"))]
    return Ok(executor
        .spawn(async move { model::process(text, normalize, trim) })
        .await);

    #[cfg(target_family = "wasm")]
    {
        let _ = executor;

        let reply = rake::web::run_worker(
            "text.worker.js",
            &rake::serde_json::json!({"text":text,"normalize":normalize,"trim":trim}),
            std::time::Duration::from_secs(10),
        )
        .await?;

        Ok(rake::serde_json::from_value(reply)?)
    }
}

struct Toolkit {
    runtime: RuntimeContext,
    workspace: Rc<Workspace>,
    preferences: Preferences,
    settings: Loader<bool, rake::platform::Error>,
    settings_applied: bool,
    trim_lines: bool,
    work: Option<AsyncAction<&'static str, Error>>,
    updates: UpdateHandle,
    deployment: Option<rake::web::Deployment>,
    web_updates: Option<Loader<Option<rake::web::DeploymentVersion>, rake::web::Error>>,
}

impl Toolkit {
    fn import(&mut self, cx: &mut Context<Self>) {
        if self.busy(cx) || self.workspace.dirty() {
            return;
        }

        let selection = self.runtime.services().platform().pick_file(cx);
        let executor = cx.background_executor().clone();
        let workspace = self.workspace.clone();
        let expected = workspace.revision();

        self.work = Some(self.runtime.action(cx, async move {
            let Some(file) = selection.await? else {
                return Ok("File selection cancelled");
            };

            let text = String::from_utf8(file.read(model::MAX_BYTES).await?)?;
            let processed = prepare(text, false, false, executor).await?;

            workspace.import(expected, Document::new(file.name().into(), processed))?;

            Ok("Text file imported")
        }));
    }

    fn normalize(&mut self, cx: &mut Context<Self>) {
        if self.busy(cx) {
            return;
        }

        let Some(document) = self.workspace.current() else {
            return;
        };

        let workspace = self.workspace.clone();
        let expected = workspace.revision();
        let executor = cx.background_executor().clone();
        let trim = self.trim_lines;

        self.work = Some(self.runtime.action(cx, async move {
            let processed = prepare(document.text.to_string(), true, trim, executor).await?;

            workspace.transform(expected, Document::new(document.name, processed))?;

            Ok("Line endings normalized")
        }));
    }

    fn export(&mut self, cx: &mut Context<Self>) {
        if self.busy(cx) {
            return;
        }

        let Some(document) = self.workspace.current() else {
            return;
        };

        let request = self.runtime.services().platform().export_file(
            &document.name,
            document.text.as_bytes().to_vec(),
            cx,
        );

        let workspace = self.workspace.clone();
        let expected = workspace.revision();

        self.work = Some(self.runtime.action(cx, async move {
            let outcome = request.await?;

            workspace.exported(expected, outcome)?;

            Ok(match outcome {
                SaveOutcome::Saved => "File saved",
                SaveOutcome::Cancelled => "Export cancelled",
                SaveOutcome::DownloadRequested => {
                    "Download requested. Confirm it saved before closing."
                }
            })
        }));
    }

    fn busy(&self, cx: &gpui::App) -> bool {
        self.work.as_ref().is_some_and(|work| work.is_running(cx))
    }
}

impl Render for Toolkit {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.settings_applied
            && let LoadStatus::Ready(trim) = self.settings.state(cx)
        {
            self.trim_lines = *trim;
            self.settings_applied = true;
        }

        let settings_error = match self.settings.state(cx) {
            LoadStatus::Failed(error) => Some(error.to_string()),
            _ => None,
        };

        let document = self.workspace.current();
        let busy = self.busy(cx);
        let dirty = self.workspace.dirty();
        let capabilities = self.runtime.services().platform().capabilities();

        let status = match self.work.as_ref().map(|work| work.state(cx)) {
            Some(LoadStatus::Loading(_)) => "Working…".into(),
            Some(LoadStatus::Ready(message)) => (*message).to_owned(),
            Some(LoadStatus::Failed(error)) => error.to_string(),
            _ => "Import a UTF-8 file up to 128 KiB. Your source is never changed automatically."
                .into(),
        };

        let update_version = self.updates.available_version(cx);
        let update_error = self.updates.error(cx);
        let can_check = self.updates.can_check(cx);
        let can_download = self.updates.can_download(cx);
        let can_restart = self.updates.can_restart(cx);

        let web_available = self
            .web_updates
            .as_ref()
            .is_some_and(|updates| matches!(updates.state(cx), LoadStatus::Ready(Some(_))));

        let web_status = self
            .web_updates
            .as_ref()
            .map(|updates| match updates.state(cx) {
                LoadStatus::Ready(Some(version)) => {
                    format!("Web version {} available", version.version)
                }
                LoadStatus::Ready(None) => "Web deployment is current".into(),
                LoadStatus::Failed(error) => error.to_string(),
                _ => "Checking Web deployment…".into(),
            });

        rake::view! {
            <main id="text-toolkit" class="flex flex-col size-full p-6 gap-3 overflow-y-scroll bg-surface text-ink" tab-navigation>
                <h1>"Text Toolkit"</h1>
                <p>{status}</p>
                <div class="flex flex-row gap-3">
                    <button id="import-file" disabled={busy || dirty || !capabilities.file_import.supported}
                        on:click={cx.listener(|this, _, _, cx| this.import(cx))}>"Import text"</button>
                    <button id="normalize-text" disabled={busy || document.is_none()}
                        on:click={cx.listener(|this, _, _, cx| this.normalize(cx))}>"Normalize line endings"</button>
                    <button id="export-file" disabled={busy || document.is_none() || !capabilities.file_export.supported}
                        on:click={cx.listener(|this, _, _, cx| this.export(cx))}>"Export"</button>
                    <button id="copy-text" disabled={busy || document.is_none() || !capabilities.clipboard_write.supported}
                        on:click={cx.listener(|this, _, _, cx| {
                            if this.busy(cx) { return; }
                            let Some(document) = this.workspace.current() else { return; };
                            let request = this.runtime.services().platform().write_clipboard(&document.text, cx);
                            this.work = Some(this.runtime.action(cx, async move { request.await?; Ok("Copied text") }));
                        })}>"Copy text"</button>
                </div>
                <button id="trim-lines" disabled={busy || !self.settings_applied}
                    on:click={cx.listener(|this, _, _, cx| {
                        if this.busy(cx) || !this.settings_applied { return; }
                        this.trim_lines = !this.trim_lines;
                        let value = if this.trim_lines { "true" } else { "false" };
                        let preferences = this.preferences.clone();
                        this.work = Some(this.runtime.action(cx, async move { preferences.set("trim-lines", value).await?; Ok("Preference saved") }));
                    })}>{format!("Trim trailing spaces: {}", if self.trim_lines { "on" } else { "off" })}</button>
                if let Some(error) = settings_error {
                    <p>{error}</p>
                    <button on:click={cx.listener(|this, _, _, cx| this.settings.retry(cx))}>"Retry preferences"</button>
                }
                if dirty {
                    <p>"Unsaved transformation. Export or discard before quitting or updating."</p>
                    <button id="discard" disabled={busy} on:click={cx.listener(|this, _, _, cx| {
                        if !this.busy(cx) { this.workspace.discard(); cx.notify(); }
                    })}>"Discard transformation"</button>
                }
                if self.workspace.download_requested() {
                    <button id="confirm-export" disabled={busy} on:click={cx.listener(|this, _, _, cx| {
                        if !this.busy(cx) { this.workspace.confirm_download(); cx.notify(); }
                    })}>"Confirm downloaded file was saved"</button>
                }
                if let Some(document) = document {
                    <h2>{document.name}</h2>
                    <p>{format!("{} bytes · {} characters · {} words · {} lines", document.statistics.bytes, document.statistics.characters, document.statistics.words, document.statistics.lines)}</p>
                    <p wrap={true} max-lines={8} ellipsis={true}>{document.text.chars().take(600).collect::<String>()}</p>
                }
                <div class="flex flex-row gap-3">
                    <button disabled={!can_check} on:click={cx.listener(|this, _, _, cx| this.updates.check(cx))}>"Check native updates"</button>
                    <button disabled={!can_download} on:click={cx.listener(|this, _, _, cx| this.updates.download(cx))}>"Download update"</button>
                    <button disabled={!can_restart} on:click={cx.listener(|this, _, _, cx| this.updates.restart(cx))}>"Restart to update"</button>
                </div>
                if let Some(version) = update_version { <p>{format!("Native version {version} available")}</p> }
                if let Some(error) = update_error { <p>{error}</p> }
                if let Some(status) = web_status {
                    <p>{status}</p>
                    <button on:click={cx.listener(|this, _, _, cx| {
                        if let Some(updates) = &mut this.web_updates { updates.retry(cx); }
                    })}>"Check Web deployment"</button>
                    <button disabled={!web_available || busy} on:click={cx.listener(|this, _, _, cx| {
                        let Some(deployment) = this.deployment.clone() else { return; };
                        let runtime = this.runtime.clone();
                        this.work = Some(this.runtime.action(cx, async move {
                            // Return after submission; awaiting quit inside this finite action would wait on itself.
                            drop(runtime.reload_web_update(&deployment, QuitOptions::default()).await?);
                            Ok("Reload requested; export/discard approval is required")
                        }));
                    })}>"Reload to update"</button>
                }
            </main>
        }
    }
}

/// Configures the same standalone application on native and browser hosts.
pub fn application() -> Result<App, rake::AppError> {
    let app = App::new("dev.rake.text-toolkit").service(Workspace::default())?;
    let workspace = app.services()?.get::<Workspace>()?;
    let guarded = workspace.clone();

    Ok(app
        .commands(commands()?)
        .theme(Colors {
            surface: gpui::rgb(0x18232b),
            ink: gpui::rgb(0xf0f4e8),
        })
        .control(rake::control::ControlConfig::FromEnvironment)
        .quit_guard(move |_| {
            let dirty = guarded.dirty();

            async move {
                if dirty {
                    Err(rake::router::GuardReason(
                        "Export or discard the transformation before quitting".into(),
                    ))
                } else {
                    Ok(())
                }
            }
        })
        .try_root_with(move |runtime, _, cx| {
            let preferences = runtime
                .services()
                .platform()
                .preferences("dev.rake.text-toolkit")
                .map_err(|error| rake::router::MountError(error.to_string()))?;

            let deployment = rake::web::Deployment::from_document()
                .map_err(|error| rake::router::MountError(error.to_string()))?;

            Ok(cx
                .new(|cx| {
                    let store = preferences.clone();

                    let settings = runtime.load(cx, move |_| {
                        let store = store.clone();

                        async move {
                            match store.get("trim-lines").await?.as_deref() {
                                Some("true") => Ok(true),
                                Some("false") | None => Ok(false),
                                Some(_) => Err(rake::platform::Error::InvalidInput(
                                    "invalid trim-lines preference".into(),
                                )),
                            }
                        }
                    });

                    let web_updates = deployment.clone().map(|deployment| {
                        runtime.load(cx, move |_| {
                            let deployment = deployment.clone();

                            async move { deployment.check().await }
                        })
                    });

                    Toolkit {
                        updates: runtime.bind_updates(cx),
                        runtime,
                        workspace: workspace.clone(),
                        preferences,
                        settings,
                        settings_applied: false,
                        trim_lines: false,
                        work: None,
                        deployment,
                        web_updates,
                    }
                })
                .into())
        }))
}

#[rake::app]
fn app() -> Result<App, rake::AppError> {
    application()
}
