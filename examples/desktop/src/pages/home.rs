//! Retained page data uses checked entry-owned work and generation-safe retry state.
use super::Destination;
use rake::gpui::{self, Context, Entity, Window, prelude::*};
use rake::prelude::*;
use rake::router::MountError;

struct WelcomeLoaded {
    token: LoadToken,
    result: Result<String, String>,
}

pub(crate) struct Home {
    page: PageContext<Destination>,
    welcome: LoadState<String>,
    load_task: Option<TaskHandle>,
    edits: u32,
    error: Option<String>,
}

impl Page<Destination> for Home {
    fn mount(
        _: &Destination,
        page: PageContext<Destination>,
        _: &mut Window,
        cx: &mut gpui::App,
    ) -> Result<Entity<Self>, MountError> {
        let view = cx.new(|_| Self {
            page: page.clone(),
            welcome: LoadState::default(),
            load_task: None,
            edits: 0,
            error: None,
        });
        let weak = view.downgrade();
        let context = cx.to_async();
        page.on_event(move |event: WelcomeLoaded| {
            let mut context = context.clone();
            weak.update(&mut context, |this, cx| {
                if this.welcome.complete(event.token, event.result) {
                    cx.notify();
                }
            })
            .map_err(|_| TaskError::OwnerDisposed)
        })
        .map_err(|error| MountError(error.to_string()))?;
        view.update(cx, |this, cx| this.load(cx));
        Ok(view)
    }
}

fn load_welcome() -> Result<String, String> {
    match rake::resources!() {
        Ok(resources) => {
            let path = resources
                .path("welcome.txt")
                .map_err(|error| error.to_string())?;
            std::fs::read_to_string(path).map_err(|error| error.to_string())
        }
        Err(rake::resources::Error::Unbundled) => {
            Ok(include_str!("../../assets/welcome.txt").into())
        }
        Err(error) => Err(error.to_string()),
    }
}

impl Home {
    fn load(&mut self, cx: &mut Context<Self>) {
        if let Some(previous) = self.load_task.take() {
            let _ = previous.cancel();
        }
        let token = self.welcome.begin();
        let background = cx.background_executor().clone();
        let work = self.page.tasks().and_then(|tasks| {
            tasks.spawn_with("welcome", move |run| async move {
                let result = background.spawn(async { load_welcome() }).await;
                run.publish(WelcomeLoaded { token, result })
            })
        });
        match work {
            Ok(task) => self.load_task = Some(task),
            Err(error) => {
                self.welcome.complete(token, Err(error.to_string()));
            }
        }
        cx.notify();
    }
}

impl Render for Home {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let loading = matches!(self.welcome.state(), LoadStatus::Loading(_));
        let failed = matches!(self.welcome.state(), LoadStatus::Failed(_));
        let welcome = match self.welcome.state() {
            LoadStatus::Idle | LoadStatus::Loading(_) => "Loading welcome message…".into(),
            LoadStatus::Ready(message) => message.clone(),
            LoadStatus::Failed(error) => format!("Could not load the welcome message: {error}"),
        };
        rake::view! {
            <section class="gap-3">
                <p>{welcome}</p>
                <button disabled={loading} on:click={cx.listener(|this, _, _, cx| this.load(cx))}>
                    {if failed { "Retry" } else { "Reload welcome message" }}
                </button>
                <p>{format!("Retained local edits: {}", self.edits)}</p>
                <button on:click={cx.listener(|this, _, _, cx| {
                    this.error = this.page.entry().set_dirty(true).err().map(|e| e.to_string());
                    if this.error.is_none() { this.edits += 1; }
                    cx.notify();
                })}>"Make an unsaved change"</button>
                <button on:click={cx.listener(|this, _, _, cx| {
                    this.error = this.page.entry().set_dirty(false).err().map(|e| e.to_string());
                    cx.notify();
                })}>"Mark clean"</button>
                <p>"Back/Forward retain this screen. Quit or update restart asks before discarding dirty state."</p>
                if let Some(error) = &self.error { <p>{error.clone()}</p> }
            </section>
        }
    }
}
