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
fn standalone_route_options_fail_instead_of_being_ignored() {
    let app = App::new("com.example.root")
        .root(|_, _, cx| {
            use rake::gpui::AppContext;
            cx.new(|_| rake::gpui::Empty).into()
        })
        .workspace_chrome(false)
        .control(rake::control::ControlConfig::FromEnvironment)
        .command::<RootCommand, _, _>(|_, _| async { Ok(()) })
        .unwrap();
    assert!(matches!(app.run(), Err(AppError::RoutingRequiresRoutes)));
}

#[derive(Clone, rake::Route)]
enum Destination {
    #[route(id = "document", path = "/documents/{id}")]
    Document(String),
}

#[test]
fn invalid_explicit_destination_fails_before_ui_startup() {
    let views = rake::router::__routes(|_: &Destination, _, _, _| {
        Err(rake::router::MountError("unused factory".into()))
    });
    let app = App::new("example.invalid-destination")
        .routes(views)
        .initial(Destination::Document(String::new()));

    assert!(matches!(app.run(), Err(AppError::Route(_))));
}

#[test]
fn route_type_transition_rejects_configuration_it_cannot_preserve() {
    let views = || {
        rake::router::__routes(|_: &Destination, _, _, _| {
            Err(rake::router::MountError("unused factory".into()))
        })
    };
    let guarded = App::new("example.order")
        .entry_guard(|_| async { rake::router::GuardResult::Allow })
        .routes(views());
    assert!(matches!(
        guarded.run(),
        Err(AppError::InvalidConfiguration(_))
    ));

    let layout = App::new("example.order")
        .layout(|_, _, cx| {
            use rake::gpui::AppContext;
            cx.new(|_| rake::gpui::Empty)
        })
        .routes(views());
    assert!(matches!(
        layout.run(),
        Err(AppError::InvalidConfiguration(_))
    ));
}
