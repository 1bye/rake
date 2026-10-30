//! Minimal distributable application using the public Rake view syntax.
use rake::gpui::prelude::*;
use rake::gpui::{Context, Rgba, Window, rgb};
use rake::prelude::*;
#[derive(Clone, rake::Theme)]
struct Colors {
    surface: Rgba,
    ink: Rgba,
}
rake::theme!(Colors);
struct Desktop;
impl Render for Desktop {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        rake::view! {
            <div class="flex flex-col items-center justify-center size-full gap-4 bg-surface text-ink">
                <div class="text-3xl">"Rake Desktop"</div>
                <div>"One application. Native distribution."</div>
            </div>
        }
    }
}
#[rake::app]
fn application() -> App {
    App::new("dev.rake.desktop-example").root(|_, _, cx| {
        rake::theme::install(
            cx,
            Colors {
                surface: rgb(0x18232b),
                ink: rgb(0xf0f4e8),
            },
        );
        cx.new(|_| Desktop).into()
    })
}
