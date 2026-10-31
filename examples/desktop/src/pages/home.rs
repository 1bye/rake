//! Route preparation returns owned data before the retained page is mounted.
use super::Destination;
use rake::gpui::{self, Context, Entity, Window, prelude::*};
use rake::prelude::*;
use rake::router::MountError;

pub(crate) struct Home {
    page: PageContext<Destination>,
    welcome: String,
    edits: u32,
    error: Option<String>,
}

impl LoadedPage<Destination> for Home {
    type Data = String;
    type Error = rake::resources::Error;

    async fn load(_: Destination, load: LoadContext) -> Result<Self::Data, Self::Error> {
        // File access is blocking; the loader itself stays on the UI event loop.
        load.background_executor()
            .spawn(async { load_welcome() })
            .await
    }

    fn mount(
        _: &Destination,
        welcome: Self::Data,
        page: PageContext<Destination>,
        _: &mut Window,
        cx: &mut gpui::App,
    ) -> Result<Entity<Self>, MountError> {
        Ok(cx.new(|_| Self {
            page,
            welcome,
            edits: 0,
            error: None,
        }))
    }
}

fn load_welcome() -> Result<String, rake::resources::Error> {
    match rake::resources!() {
        Ok(resources) => Ok(std::fs::read_to_string(resources.path("welcome.txt")?)?),
        Err(rake::resources::Error::Unbundled) => {
            Ok(include_str!("../../assets/welcome.txt").into())
        }
        Err(error) => Err(error),
    }
}

impl Render for Home {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        rake::view! {
            <section class="gap-3">
                <p>{self.welcome.clone()}</p>
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
