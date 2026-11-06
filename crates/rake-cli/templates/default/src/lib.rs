//! Shared native/browser app: retained routes, a persistent shell and an agent command.

use rake::gpui::{self, Context, Entity, Window, prelude::*};
use rake::prelude::*;
use rake::router::MountError;

rake::theme!(rake::theme::Semantic);

#[derive(Clone, rake::Route)]
enum Destination {
    #[route(id = "home", path = "/")]
    Home,
    #[route(id = "about", path = "/about")]
    About,
}

#[rake::data(schema)]
#[derive(rake::Command)]
#[command(id = "app.greet", output = String, capability = Headless)]
struct Greet {
    name: String,
}

struct Home {
    count: usize,
}

impl Page<Destination> for Home {
    fn mount(
        _: &Destination,
        _: PageContext<Destination>,
        _: &mut Window,
        cx: &mut gpui::App,
    ) -> Result<Entity<Self>, MountError> {
        Ok(cx.new(|_| Self { count: 0 }))
    }
}

impl Render for Home {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        rake::view! {
            <section class="gap-4">
                <h2>"Hello, Rake!"</h2>
                <p>"One Rust app for desktop and the browser."</p>
                <button
                    id="increment"
                    class="px-4 py-2 rounded-md bg-primary text-primary-foreground"
                    on:click={
                        cx.listener(|this, _, _, cx| {
                            this.count += 1;
                            cx.notify();
                        })
                    }
                >
                    "Increment"
                </button>
                <p>{format!("Count: {}", self.count)}</p>
            </section>
        }
    }
}

struct About;

impl Page<Destination> for About {
    fn mount(
        _: &Destination,
        _: PageContext<Destination>,
        _: &mut Window,
        cx: &mut gpui::App,
    ) -> Result<Entity<Self>, MountError> {
        Ok(cx.new(|_| Self))
    }
}

impl Render for About {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        rake::view! {
            <section class="gap-4">
                <h2>"About"</h2>
                <p>"The layout stays mounted while pages change. Back restores the counter."</p>
                <p>"Agents can discover and call app.greet through rake ctl."</p>
            </section>
        }
    }
}

struct Shell {
    layout: LayoutContext<Destination>,
    visits: usize,
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let outlet = self.layout.outlet();

        rake::view! {
            <main class="size-full p-6 gap-4 bg-background text-foreground" tab-navigation>
                <h1>"{{name}}"</h1>
                <nav class="flex-row gap-4">
                    <button
                        id="home"
                        on:click={
                            cx.listener(|this, _, _, cx| {
                                this.visits += 1;
                                drop(
                                    this.layout
                                        .navigator()
                                        .open(Destination::Home, OpenTarget::CurrentTab)
                                );
                                cx.notify();
                            })
                        }
                    >
                        "Home"
                    </button>
                    <button
                        id="about"
                        on:click={
                            cx.listener(|this, _, _, cx| {
                                this.visits += 1;
                                drop(
                                    this.layout
                                        .navigator()
                                        .open(Destination::About, OpenTarget::CurrentTab)
                                );
                                cx.notify();
                            })
                        }
                    >
                        "About"
                    </button>
                    <button
                        id="back"
                        on:click={
                            cx.listener(|this, _, _, _| {
                                if let Ok(tab) = this.layout.navigator().current_tab() {
                                    drop(tab.back());
                                }
                            })
                        }
                    >
                        "Back"
                    </button>
                </nav>
                <section class="flex-1">{outlet}</section>
                <footer>{format!("Navigation clicks: {}", self.visits)}</footer>
            </main>
        }
    }
}

#[rake::app]
fn app() -> Result<App<Destination>, rake::control::CommandError> {
    App::new("{{application_id}}")
        .theme(rake::theme::Semantic::dark())
        .routes(rake::routes! {
            Destination::Home => Home,
            Destination::About => About,
        })
        .layout(|layout, _, cx| cx.new(|_| Shell { layout, visits: 0 }))
        .initial(Destination::Home)
        .workspace_chrome(false)
        .control(rake::control::ControlConfig::FromEnvironment)
        .command::<Greet, _, _>(|args, _| async move { Ok(format!("Hello, {}!", args.name)) })
}
