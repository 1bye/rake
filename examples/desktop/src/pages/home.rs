//! The home page owns resource loading, retained edits and its route context.
use super::Destination;
use crate::{AppInfo, shell::Shell};
use rake::gpui::{self, Context, Entity, Window, prelude::*};
use rake::prelude::*;
use rake::router::MountError;
use std::rc::Rc;

pub(crate) struct Home {
    page: PageContext<Destination>,
    app: Rc<AppInfo>,
    welcome: String,
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
        let app = page
            .services()
            .get::<AppInfo>()
            .map_err(|error| MountError(error.to_string()))?;
        let welcome = match rake::resources!() {
            Ok(resources) => {
                let path = resources
                    .path("welcome.txt")
                    .map_err(|error| MountError(error.to_string()))?;
                std::fs::read_to_string(path).map_err(|error| MountError(error.to_string()))?
            }
            Err(rake::resources::Error::Unbundled) => {
                include_str!("../../assets/welcome.txt").into()
            }
            Err(error) => return Err(MountError(error.to_string())),
        };

        Ok(cx.new(|_| Self {
            page,
            app,
            welcome,
            edits: 0,
            error: None,
        }))
    }
}

impl Render for Home {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = rake::view! {
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
        };

        rake::view! {
            <Shell page={self.page.clone()} app={Rc::clone(&self.app)} body={body.into_any_element()}/>
        }
    }
}
