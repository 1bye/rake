//! Small native application with routing, shared services and distribution.
use rake::prelude::*;

mod pages;
mod shell;

#[derive(Clone, rake::Theme)]
struct Colors {
    surface: rake::gpui::Rgba,
    ink: rake::gpui::Rgba,
    accent: rake::gpui::Rgba,
}

rake::theme!(Colors);

struct AppInfo {
    name: &'static str,
    version: &'static str,
}

#[rake::app]
fn application() -> Result<App<pages::Destination>, rake::AppError> {
    Ok(App::new("dev.rake.starter")
        .service(AppInfo {
            name: "Rake Starter",
            version: env!("CARGO_PKG_VERSION"),
        })?
        .theme(Colors {
            surface: rake::gpui::rgb(0x18232b),
            ink: rake::gpui::rgb(0xf0f4e8),
            accent: rake::gpui::rgb(0x365849),
        })
        .routes(rake::routes! {
            pages::Destination::Home => pages::Home,
            pages::Destination::Updates => pages::Updates,
        })
        .runtime_config(RouterConfig {
            history_capacity: Some(64),
            ..Default::default()
        })
        .try_layout(shell::Shell::mount)
        .initial(pages::Destination::Home)
        .workspace_chrome(false)
        .control(rake::control::ControlConfig::FromEnvironment))
}
