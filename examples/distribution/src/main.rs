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
struct Desktop {
    rendered: bool,
}
impl Render for Desktop {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        if cfg!(feature = "native-acceptance") && !self.rendered {
            self.rendered = true;
            println!("RAKE_NATIVE_RENDER");
            let resources = rake::resources!().expect("installed acceptance resources");
            let welcome = std::fs::read_to_string(resources.path("welcome.txt").unwrap()).unwrap();
            window.on_next_frame(move |_, cx| {
                println!("RAKE_NATIVE_FRAME:{}", welcome.trim());
                cx.quit();
            });
        }
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
    if cfg!(feature = "native-acceptance") {
        println!("RAKE_NATIVE_START:{}", env!("CARGO_PKG_VERSION"));
    }
    App::new("dev.rake.desktop-example").root(|_, _, cx| {
        if cfg!(feature = "native-acceptance") {
            println!("RAKE_NATIVE_WINDOW");
        }
        rake::theme::install(
            cx,
            Colors {
                surface: rgb(0x18232b),
                ink: rgb(0xf0f4e8),
            },
        );
        cx.new(|_| Desktop { rendered: false }).into()
    })
}
