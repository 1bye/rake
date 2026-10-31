//! One persistent layout per window; route history owns only the page bodies.
use crate::{AppInfo, pages::Destination};
use rake::gpui::{self, Context, Entity, Window, prelude::*};
use rake::prelude::*;
use rake::router::MountError;
use std::rc::Rc;

pub(crate) struct Shell {
    layout: LayoutContext<Destination>,
    app: Rc<AppInfo>,
    navigation_clicks: usize,
}

impl Shell {
    pub(crate) fn mount(
        layout: LayoutContext<Destination>,
        _: &mut Window,
        cx: &mut gpui::App,
    ) -> Result<Entity<Self>, MountError> {
        let app = layout
            .services()
            .get::<AppInfo>()
            .map_err(|error| MountError(error.to_string()))?;
        Ok(cx.new(|_| Self {
            layout,
            app,
            navigation_clicks: 0,
        }))
    }

    fn travel(&self, forward: bool) {
        if let Ok(tab) = self.layout.navigator().current_tab() {
            drop(if forward { tab.forward() } else { tab.back() });
        }
    }
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ready = self.layout.navigator().current_tab().is_ok();
        let outlet = self.layout.outlet();

        rake::view! {
            <main class="size-full p-6 gap-4 bg-surface text-ink" tab-navigation>
                <h1>{self.app.name}</h1>
                <nav class="flex flex-row gap-3">
                    <button disabled={!ready} class="px-3 py-2 rounded-md bg-accent"
                        on:click={cx.listener(|this, _, _, cx| {
                            this.navigation_clicks += 1;
                            drop(this.layout.navigator().open(Destination::Home, OpenTarget::CurrentTab));
                            cx.notify();
                        })}>"Home"</button>
                    <button disabled={!ready} class="px-3 py-2 rounded-md bg-accent"
                        on:click={cx.listener(|this, _, _, cx| {
                            this.navigation_clicks += 1;
                            drop(this.layout.navigator().open(Destination::Updates, OpenTarget::CurrentTab));
                            cx.notify();
                        })}>"Updates"</button>
                    <button disabled={!ready} on:click={cx.listener(|this, _, _, _| this.travel(false))}>"Back"</button>
                    <button disabled={!ready} on:click={cx.listener(|this, _, _, _| this.travel(true))}>"Forward"</button>
                    <button on:click={cx.listener(|this, _, _, _| {
                        drop(this.layout.runtime().quit(QuitOptions::default()));
                    })}>"Quit"</button>
                </nav>
                <section class="flex-1 overflow-hidden">{outlet}</section>
                <footer>{format!("Version {} · Navigation clicks {}", self.app.version, self.navigation_clicks)}</footer>
            </main>
        }
    }
}
