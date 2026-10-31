//! Ordinary reusable view composition; the shell does not implement Page.
use crate::{AppInfo, pages::Destination};
use rake::gpui::{self, AnyElement, Window, prelude::*};
use rake::prelude::*;
use std::rc::Rc;

#[derive(gpui::IntoElement)]
pub(crate) struct Shell {
    pub page: PageContext<Destination>,
    pub app: Rc<AppInfo>,
    pub body: AnyElement,
}

impl RenderOnce for Shell {
    fn render(self, _: &mut Window, _: &mut gpui::App) -> impl IntoElement {
        let home = self.page.tab();
        let updates = self.page.tab();
        let back = self.page.tab();
        let forward = self.page.tab();
        let quit = self.page.navigator();

        rake::view! {
            <main class="size-full p-6 gap-4 bg-surface text-ink" tab-navigation>
                <h1>{self.app.name}</h1>
                <nav class="flex flex-row gap-3">
                    <button class="px-3 py-2 rounded-md bg-accent"
                        on:click={move |_, _, _| { drop(home.navigate(Destination::Home)); }}>"Home"</button>
                    <button class="px-3 py-2 rounded-md bg-accent"
                        on:click={move |_, _, _| { drop(updates.navigate(Destination::Updates)); }}>"Updates"</button>
                    <button on:click={move |_, _, _| { drop(back.back()); }}>"Back"</button>
                    <button on:click={move |_, _, _| { drop(forward.forward()); }}>"Forward"</button>
                    <button on:click={move |_, _, _| { drop(quit.quit(QuitOptions::default())); }}>"Quit"</button>
                </nav>
                {self.body}
                <footer>{format!("Version {}", self.app.version)}</footer>
            </main>
        }
    }
}
