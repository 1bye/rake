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
    let preferences = rake::platform::Platform::default()
        .preferences("dev.rake.starter")
        .map_err(|error| rake::AppError::Window(error.to_string()))?;

    #[cfg(all(feature = "native-smoke", not(target_family = "wasm")))]
    let preferences = std::env::var_os("RAKE_DESKTOP_SMOKE_PREFERENCES")
        .map(rake::platform::Preferences::at)
        .unwrap_or(preferences);

    Ok(App::new("dev.rake.starter")
        .window_state(rake::window_state::WindowState::new(preferences))
        .window_options(window_options())
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
            pages::Destination::Home => loaded(pages::Home),
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

fn window_options() -> rake::gpui::WindowOptions {
    #[cfg(feature = "native-smoke")]
    if std::env::var_os("RAKE_DESKTOP_SMOKE_ALTERNATE_BOUNDS").is_some() {
        use rake::gpui::{Bounds, WindowBounds, point, px, size};

        return rake::gpui::WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(200.), px(150.)),
                size(px(720.), px(480.)),
            ))),
            ..Default::default()
        };
    }

    rake::gpui::WindowOptions::default()
}
