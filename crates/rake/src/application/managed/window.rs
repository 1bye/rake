//! Repeatable standalone windows share the retained background scheduler.
use super::{
    ManagedConfig, ManagedDriver, ManagedRoot, prepare_background, spawn_background_driver,
};

use crate::runtime::RuntimeContext;
use crate::{AppError, Services};
use gpui::AppContext as _;
use std::{cell::RefCell, rc::Rc};

struct MainWindow {
    context: RuntimeContext,
    navigator: crate::router::Navigator<crate::router::NoRoute>,
    options: super::super::WindowOptionsFactory,
    mount: super::super::MainWindow,
    close_policy: super::ClosePolicy,
    current: Option<gpui::AnyWindowHandle>,
    opening: bool,
}

impl gpui::Global for MainWindow {}

pub(in crate::application) fn install_main_window(
    options: super::super::WindowOptionsFactory,
    mount: super::super::MainWindow,
    setup: Option<super::super::Background>,
    services: Services,
    config: ManagedConfig,
    cx: &mut gpui::App,
) {
    let runtime = prepare_background(&services, &config, cx);
    let context = runtime.context();
    let navigator = runtime.router.navigator();
    let quit_router = runtime.router.clone();

    let quit = cx.on_app_quit(move |_| {
        quit_router.shutdown();

        async {}
    });

    let start_hidden = setup.is_some();

    cx.set_global(MainWindow {
        context: context.clone(),
        navigator,
        options,
        mount,
        close_policy: config.close_policy,
        current: None,
        opening: false,
    });
    let driver = spawn_background_driver(runtime, config, cx);

    cx.set_global(ManagedDriver {
        _driver: driver,
        _quit: quit,
    });

    if let Some(setup) = setup {
        setup(context, cx);
    }

    if !start_hidden && let Err(error) = show_main_window(cx) {
        // Runtime stays alive so a tray action can retry a failed mount.
        crate::error!(%error, "main window could not open");
    }
}

pub(in crate::application) fn show_main_window(cx: &mut gpui::App) -> Result<(), AppError> {
    let host = cx
        .try_global::<MainWindow>()
        .ok_or(AppError::InvalidConfiguration(
            "no main_window factory is registered",
        ))?;

    host.context
        .client()
        .map_err(|error| AppError::Window(error.to_string()))?;

    if host.opening {
        return Err(AppError::Window("main window is already opening".into()));
    }

    let current = host.current;

    if let Some(current) = current
        && current
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        cx.activate(true);

        return Ok(());
    }

    let host = cx.global::<MainWindow>();
    let context = host.context.clone();

    #[cfg(feature = "live")]
    let diagnostics = context.services().diagnostics();

    let router = host
        .navigator
        .runtime()
        .map_err(|error| AppError::Window(error.to_string()))?;

    let mount = host.mount.clone();
    let options = host.options.clone();
    let policy = host.close_policy;

    cx.global_mut::<MainWindow>().opening = true;
    let failure = Rc::new(RefCell::new(None));
    let failed = failure.clone();

    let mut options = options();

    let placement =
        crate::window_state::native::prepare(crate::router::WindowId(1), &mut options, cx);

    let opened = cx.open_window(options, move |window, cx| {
        let content = match mount(context, window, cx) {
            Ok(content) => content,
            Err(error) => {
                *failed.borrow_mut() = Some(error);
                let empty: gpui::AnyView = cx.new(|_| gpui::Empty).into();

                return cx.new(|cx| rake_ui::Root::new(empty, window, cx));
            }
        };

        crate::window_state::native::observe(placement, window, cx);

        #[cfg(feature = "live")]
        crate::reload::live::install(
            diagnostics,
            cx,
            Some(crate::reload::live::ReloadOwner::from_router(
                router.clone(),
            )),
        );

        match policy {
            super::ClosePolicy::Quit => router.observe_host(window, cx),
            super::ClosePolicy::KeepRunning => router.observe_root_host(window, cx),
        }

        let view = cx.new(|cx| {
            let observed = router.clone();

            let observer = cx.spawn(async move |view, cx| {
                loop {
                    let revision = observed.snapshot().revision;

                    observed.changed(revision).await;

                    if view
                        .update(cx, |root: &mut ManagedRoot, cx| {
                            gpui::App::notify(cx, root.content.entity_id());
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            });

            ManagedRoot {
                content,
                router,
                close_policy: policy,
                _observer: Some(observer),
            }
        });

        cx.new(|cx| rake_ui::Root::new(view, window, cx))
    });

    cx.global_mut::<MainWindow>().opening = false;
    let handle = opened.map_err(|error| AppError::Window(error.to_string()))?;

    if let Some(error) = failure.borrow_mut().take() {
        let _ = handle.update(cx, |_, window, _| window.remove_window());

        return Err(AppError::Mount(error));
    }

    cx.global_mut::<MainWindow>().current = Some(handle.into());
    cx.activate(true);

    Ok(())
}

#[cfg(all(test, feature = "view-test-support"))]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// Real window factory/scheduler and registry; injected native creation failure.
    #[cfg(feature = "tray")]
    #[test]
    fn optional_tray_failure_opens_main_window_but_required_failure_stops_startup() {
        use crate::tray::{EventKind, HostPolicy, Image, ImageRepresentation, Tray};

        for policy in [HostPolicy::Wait, HostPolicy::Require] {
            let app = gpui::TestAppContext::single();
            let errors = super::super::super::startup::StartupErrors::default();
            let events = Rc::new(RefCell::new(Vec::new()));
            let received = events.clone();

            app.update(|cx| {
                rake_ui::init(cx);
                let base = crate::App::new("tray.startup-test").services().unwrap();

                let spec = Tray::new("main").host_policy(policy).icon(Image::new(
                    ImageRepresentation::rgba(1, 1, vec![255; 4]).unwrap(),
                ));

                let failure = crate::tray::Error::Native {
                    operation: "create",
                    message: "fixture rejected icon".into(),
                    source: None,
                };

                let host = rake_tray::Host::with_creation_failure("tray.startup-test", failure);

                let services = crate::tray::install_for_tests(
                    base,
                    vec![spec],
                    vec![Box::new(move |event, _| {
                        received.borrow_mut().push(event.clone())
                    })],
                    errors.clone(),
                    host,
                    cx,
                )
                .unwrap();

                let config = ManagedConfig {
                    config: Default::default(),
                    control: crate::control::ControlConfig::Disabled,
                    commands: Default::default(),
                    application_id: "tray.startup-test".into(),
                    close_policy: super::super::ClosePolicy::KeepRunning,
                    quit_guards: Vec::new(),
                };

                install_main_window(
                    Rc::new(gpui::WindowOptions::default),
                    Rc::new(|_, _, cx| Ok(cx.new(|_| gpui::Empty).into())),
                    None,
                    services,
                    config,
                    cx,
                );
            });
            app.run_until_parked();

            match policy {
                HostPolicy::Wait => {
                    assert!(
                        !errors.failed(),
                        "optional native failure must not poison startup"
                    );
                    assert_eq!(app.windows().len(), 1, "main window must remain open");
                    assert!(events.borrow().iter().any(|event| matches!(&event.kind, EventKind::OperationFailed(message) if message.contains("fixture rejected icon"))), "optional failure must reach on_tray_event");
                }
                HostPolicy::Require => {
                    assert!(matches!(errors.finish(), Err(crate::AppError::Tray(_))));
                    assert!(
                        app.update(|cx| cx.try_global::<crate::tray::StartupStopped>().is_some())
                    );
                    assert!(
                        app.windows().is_empty(),
                        "required startup failure must close the application UI"
                    );
                }
            }
        }
    }

    #[cfg(feature = "tray")]
    #[test]
    fn unavailable_optional_tray_host_reports_without_poisoning_startup() {
        let app = gpui::TestAppContext::single();
        let errors = super::super::super::startup::StartupErrors::default();
        let received = Rc::new(Cell::new(false));
        let seen = received.clone();

        app.update(|cx| {
            let services = crate::App::new("tray.host-test").services().unwrap();

            crate::tray::install_for_tests(
                services,
                vec![crate::tray::Tray::new("optional")],
                vec![Box::new(move |event, _| {
                    seen.set(
                        event.generation == 0
                            && matches!(event.kind, crate::tray::EventKind::OperationFailed(_)),
                    )
                })],
                errors.clone(),
                Err(crate::tray::Error::HostUnavailable),
                cx,
            )
            .unwrap();
        });
        app.run_until_parked();
        assert!(received.get());
        assert!(!errors.failed());
    }

    // Real scheduler/factory ownership; GPUI TestPlatform supplies physical windows.
    #[test]
    fn failed_mount_and_close_reopen_keep_one_runtime() {
        let app = gpui::TestAppContext::single();
        let attempts = Rc::new(Cell::new(0));
        let ids = Rc::new(RefCell::new(Vec::new()));
        let seen = ids.clone();
        let count = attempts.clone();

        app.update(|cx| {
            rake_ui::init(cx);
            let services = crate::App::new("tray.reopen").services().unwrap();

            let config = ManagedConfig {
                config: crate::router::RouterConfig::default(),
                control: crate::control::ControlConfig::Disabled,
                commands: crate::control::Commands::new(),
                application_id: "tray.reopen".into(),
                close_policy: super::super::ClosePolicy::KeepRunning,
                quit_guards: Vec::new(),
            };

            install_main_window(
                Rc::new(gpui::WindowOptions::default),
                Rc::new(move |context, _, cx| {
                    seen.borrow_mut()
                        .push(context.client().unwrap().info().runtime_id);
                    count.set(count.get() + 1);

                    if count.get() == 1 {
                        return Err(crate::router::MountError("retry".into()));
                    }

                    Ok(cx.new(|_| gpui::Empty).into())
                }),
                Some(Box::new(|_, _| {})),
                services,
                config,
                cx,
            );
            assert!(show_main_window(cx).is_err());
            show_main_window(cx).unwrap();
            show_main_window(cx).unwrap();
        });
        assert_eq!(attempts.get(), 2, "focusing must not remount");
        let windows = app.windows();

        assert_eq!(windows.len(), 1);
        app.update(|cx| {
            windows[0]
                .update(cx, |_, window, _| window.remove_window())
                .unwrap()
        });
        app.update(|cx| show_main_window(cx).unwrap());
        assert_eq!(attempts.get(), 3);
        assert!(ids.borrow().iter().all(|id| *id == ids.borrow()[0]));
        app.update(|cx| {
            cx.global::<MainWindow>()
                .navigator
                .runtime()
                .unwrap()
                .shutdown();
            assert!(show_main_window(cx).is_err());
        });
    }
}
