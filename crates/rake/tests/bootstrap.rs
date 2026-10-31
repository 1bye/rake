//! Covers: application configuration and explicit diagnostics conflicts before UI startup.
//! Real: application services, HTTP configuration and tracing initialization.
//! Doubles: none.
//! Requires: no display, GPU or external services.

#![cfg(feature = "gpui")]

use rake::diagnostics::{DiagnosticsConfig, DiagnosticsError};
use rake::http::HttpConfig;
use rake::{App, AppError};

#[test]
fn invalid_apps_fail_before_starting_the_host() {
    assert!(matches!(App::new("").run(), Err(AppError::InvalidId)));
    assert!(matches!(
        App::new("com.example.control")
            .control(rake::control::ControlConfig::FromEnvironment)
            .run(),
        Err(AppError::MissingRoot)
    ));
    assert!(matches!(
        App::new("com.example.app").run(),
        Err(AppError::MissingRoot)
    ));
    assert!(matches!(
        App::new("com.example.app")
            .http(HttpConfig::new().base_url("invalid"))
            .run(),
        Err(AppError::Http(_))
    ));
}

#[test]
fn automatic_subscriber_conflicts_are_reported() {
    DiagnosticsConfig::new()
        .install()
        .expect("first installation");
    let conflict = DiagnosticsConfig::new().install();

    assert!(matches!(
        conflict,
        Err(DiagnosticsError::SubscriberAlreadyInstalled)
    ));
}

#[rake::data(schema)]
#[derive(rake::Command)]
#[command(id = "standalone.unsupported")]
struct RootCommand;

#[test]
fn root_inspection_does_not_enable_registered_application_commands() {
    let app = App::new("com.example.root")
        .root(|_, _, cx| {
            use rake::gpui::AppContext;

            cx.new(|_| rake::gpui::Empty).into()
        })
        .control(rake::control::ControlConfig::FromEnvironment)
        .command::<RootCommand, _, _>(|_, _| async { Ok(()) })
        .unwrap();

    assert!(matches!(app.run(), Err(AppError::ControlRequiresRouter)));
}
