//! Application-owned system tray icons, native menus and platform capabilities.
//!
//! Enable `tray`, declare items with [`crate::App::tray`], and access the shared
//! weak collection through [`crate::runtime::RuntimeContext::trays`]. Control
//! handles do not retain the runtime or unregister an item when dropped.

pub use rake_tray::{
    Accelerator, Anchor, Animation, Applied, Backend, BalloonEvent, Button, Capabilities,
    CheckState, Collection, CoordinateSpace, Corner, Error, Event, EventKind, Feature, Handle,
    HostPolicy, Image, ImageRepresentation, Input, Interaction, ItemKind, Menu, MenuItem,
    Modifiers, Point, PointerAction, Rect, Registration, Rendering, Request, State, Support, Tray,
    linux, macos, windows,
};

mod popup;
pub use popup::{
    Popup, PopupFallback, PopupPlacement, PopupRetention, dismiss_popup, toggle_popup,
};

use crate::application::startup::StartupErrors;
use crate::{AppError, Services};
use std::{cell::RefCell, rc::Rc};

pub(crate) type Handler = Box<dyn FnMut(&Event, &mut gpui::App)>;
type Dispatch = Rc<
    dyn Fn(
        String,
        serde_json::Value,
    ) -> Result<crate::control::Reply, crate::control::ControlError>,
>;

type Quit = Rc<dyn Fn() -> crate::router::Operation<crate::quit::QuitResult>>;
#[derive(Clone, Default)]
struct Dispatcher {
    command: Rc<RefCell<Option<Dispatch>>>,
    quit: Rc<RefCell<Option<Quit>>>,
}

pub(crate) fn bind<R: crate::router::Route>(
    navigator: crate::router::Navigator<R>,
    services: &Services,
) {
    if let Ok(dispatcher) = services.get::<Dispatcher>() {
        let quitting = navigator.clone();
        *dispatcher.quit.borrow_mut() = Some(Rc::new(move || quitting.quit(Default::default())));
        *dispatcher.command.borrow_mut() = Some(Rc::new(move |id, arguments| {
            let router = navigator
                .runtime()
                .map_err(|_| crate::control::ControlError::RuntimeShutdown)?;
            router.client().dispatch_local_command(id, arguments)
        }));
    }
}

pub(crate) fn command_handler<C: crate::control::ApplicationCommand>(
    tray: &str,
    item: &str,
    command: C,
) -> Result<Handler, crate::control::CommandError> {
    let tray = tray.to_owned();
    let item = item.to_owned();
    let command_id = C::id();
    let arguments = serde_json::to_value(command)
        .map_err(|e| crate::control::CommandError::Arguments(e.to_string()))?;
    Ok(Box::new(move |event, cx| {
        if event.id != tray || !matches!(&event.kind, EventKind::MenuSelected(id) if id == &item) {
            return;
        }
        let dispatcher = cx.global::<Owner>().dispatch.clone();
        let dispatch = dispatcher.command.borrow().clone();
        let result = dispatch
            .ok_or(crate::control::ControlError::RuntimeShutdown)
            .and_then(|dispatch| dispatch(command_id.clone(), arguments.clone()));
        if let Err(error) = result {
            crate::error!(%error, "tray command rejected");
        }
    }))
}

async fn quit_from_removal(
    quit: Option<Quit>,
    restore: impl FnOnce() -> Request<Applied>,
) -> Result<(), Error> {
    let committed = match quit {
        Some(quit) => matches!(
            quit().wait().await,
            Ok(crate::router::Outcome::Committed(_))
        ),
        None => false,
    };
    if !committed {
        restore().await?;
    }
    Ok(())
}

struct Owner {
    host: rake_tray::Host,
    dispatch: Dispatcher,
    popups: popup::Popups,
    animations: std::collections::BTreeMap<String, gpui::Task<()>>,
    removal_quits: std::collections::BTreeMap<String, gpui::Task<()>>,
    _startup: gpui::Task<()>,
    _operations: gpui::Task<()>,
    _events: gpui::Task<()>,
    _quit: gpui::Subscription,
}

impl gpui::Global for Owner {}

pub(crate) fn stop(cx: &mut gpui::App) {
    #[cfg(target_os = "macos")]
    if let Err(error) = rake_updates::stop_application_run_loop() {
        crate::error!(%error, "native tray startup could not stop the host loop");
    }
    #[cfg(not(target_os = "macos"))]
    cx.quit();
    #[cfg(target_os = "macos")]
    let _ = cx;
}

pub(crate) fn install(
    application: &str,
    services: Services,
    specs: Vec<Tray>,
    handlers: Vec<Handler>,
    popups: Vec<(String, Popup)>,
    errors: StartupErrors,
    cx: &mut gpui::App,
) -> Result<Services, Error> {
    install_with(
        Installation {
            application: application.into(),
            specs,
            handlers,
            popups,
            errors,
        },
        services,
        rake_tray::Host::new,
        stop,
        cx,
    )
}

struct Installation {
    application: String,
    specs: Vec<Tray>,
    handlers: Vec<Handler>,
    popups: Vec<(String, Popup)>,
    errors: StartupErrors,
}

fn configuration_failure(error: &Error) -> bool {
    matches!(error, Error::InvalidInput(_) | Error::Duplicate(_))
}

fn report_unregistered_failure(
    id: String,
    error: &Error,
    handlers: &mut [Handler],
    cx: &mut gpui::App,
) {
    crate::error!(tray = %id, %error, "optional tray could not register");
    let event = Event {
        id,
        generation: 0,
        menu_revision: None,
        kind: EventKind::OperationFailed(error.to_string()),
        anchor: None,
        input: Input::default(),
    };
    for handler in handlers {
        handler(&event, cx);
    }
}

fn install_with(
    installation: Installation,
    services: Services,
    create_host: impl FnOnce(&str) -> Result<rake_tray::Host, Error>,
    stop_host: fn(&mut gpui::App),
    cx: &mut gpui::App,
) -> Result<Services, Error> {
    let Installation {
        application,
        specs,
        mut handlers,
        popups,
        errors,
    } = installation;
    #[cfg(target_family = "wasm")]
    if specs.is_empty() {
        return Ok(services);
    }

    let mut ids = std::collections::BTreeSet::new();
    for spec in &specs {
        spec.validate()?;
        if !ids.insert(spec.id()) {
            return Err(Error::Duplicate(spec.id().into()));
        }
    }

    for (id, _) in &popups {
        if !specs
            .iter()
            .any(|spec| spec.id() == id && spec.primary_interaction() == Interaction::Popup)
        {
            return Err(Error::InvalidInput(
                "popup attachment requires a declared tray with Popup interaction".into(),
            ));
        }
    }
    let has_popups = !popups.is_empty();
    let popups = popup::Popups::new(popups)?;
    let host = match create_host(&application) {
        Ok(host) => host,
        Err(error)
            if !configuration_failure(&error)
                && specs
                    .iter()
                    .all(|spec| spec.host_requirement() == HostPolicy::Wait) =>
        {
            cx.defer(move |cx| {
                for spec in specs {
                    report_unregistered_failure(spec.id().into(), &error, &mut handlers, cx);
                }
            });
            return Ok(services);
        }
        Err(error) => return Err(error),
    };
    if has_popups {
        cx.set_quit_mode(gpui::QuitMode::Explicit);
    }
    let handlers = Rc::new(RefCell::new(handlers));
    let collection = host.collection();
    let dispatch = Dispatcher::default();
    let registry = services
        .registry()
        .with(collection.clone())
        .and_then(|registry| registry.with(dispatch.clone()))
        .map_err(|error| Error::InvalidInput(error.to_string()))?;
    let services = Services::with_platform(
        services.http(),
        services.diagnostics(),
        registry,
        services.platform(),
    );
    let work = host.drive();
    let operations = cx.spawn(async move |_| {
        if let Err(error) = work.await {
            crate::error!(%error, "tray operation driver stopped");
        }
    });
    let requests = specs
        .into_iter()
        .map(|spec| {
            let id = spec.id().to_owned();
            let policy = spec.host_requirement();
            let request = collection.create(spec);
            let reserved = collection.get(&id).is_ok();
            (id, policy, reserved, request)
        })
        .collect::<Vec<_>>();
    let startup_handlers = handlers.clone();
    let startup = cx.spawn(async move |cx| {
        for (id, policy, reserved, request) in requests {
            if let Err(error) = request.await {
                if policy == HostPolicy::Wait && !configuration_failure(&error) {
                    // Admitted work reports via the registry event stream. Rejections
                    // before reservation have no generation and need direct delivery.
                    if !reserved {
                        cx.update(|cx| {
                            report_unregistered_failure(
                                id,
                                &error,
                                &mut startup_handlers.borrow_mut(),
                                cx,
                            )
                        });
                    }
                    continue;
                }
                let shutdown = cx.update(|cx| {
                    errors.record(AppError::Tray(error));
                    cx.global::<Owner>().host.shutdown()
                });
                if let Err(error) = shutdown.await {
                    crate::error!(%error, "tray startup rollback failed");
                }
                cx.update(stop_host);
                return;
            }
        }
    });
    let quit = cx.on_app_quit(|cx| {
        popup::dismiss_all(cx);
        cx.global_mut::<Owner>().animations.clear();
        cx.global_mut::<Owner>().removal_quits.clear();
        let shutdown = cx.global::<Owner>().host.shutdown();
        async move {
            if let Err(error) = shutdown.await {
                crate::error!(%error, "native tray shutdown failed");
            }
        }
    });
    let events = cx.spawn(async move |cx| {
        while let Ok(event) = collection.next_event().await {
            cx.update(|cx| {
                if matches!(
                    event.kind,
                    EventKind::Removed
                        | EventKind::Hidden
                        | EventKind::HostChanged(false)
                        | EventKind::VisibilityChanged(false)
                ) {
                    popup::removed(&event.id, event.generation, cx);
                    cx.global_mut::<Owner>().animations.remove(&event.id);
                    if matches!(event.kind, EventKind::Removed) { cx.global_mut::<Owner>().removal_quits.remove(&event.id); }
                }
                if let Ok(handle) = collection.get(&event.id)
                    && let Ok(spec) = handle.specification()
                {
                    if let EventKind::MenuSelected(id) = &event.kind
                        && spec
                            .menu_description()
                            .and_then(|menu| menu.find(id))
                            .is_some_and(|item| matches!(item.kind, ItemKind::Quit))
                    {
                        cx.dispatch_action(&crate::quit::RequestQuit);
                    }
                    if matches!(event.kind, EventKind::RemovalRequestedQuit) {
                        let quit = cx.global::<Owner>().dispatch.quit.borrow().clone();
                        let item = handle.clone();
                        let task = cx.spawn(async move |_| {
                            if let Err(error) = quit_from_removal(quit, move || item.show()).await {
                                crate::error!(%error, "removed tray item could not be restored after declined quit");
                            }
                        });
                        cx.global_mut::<Owner>().removal_quits.insert(event.id.clone(), task);
                    }
                    if matches!(event.kind, EventKind::Activate)
                        && spec.primary_interaction() == Interaction::Popup
                        && let Err(error) = toggle_popup(&event.id, cx)
                    {
                        crate::error!(%error, "tray popup could not open");
                    }
                    let menu_gesture = matches!(event.kind, EventKind::ContextMenu)
                        || (matches!(event.kind, EventKind::Activate)
                            && spec.primary_interaction() == Interaction::NativeMenu);
                    if menu_gesture
                        && spec.menu_description().is_some()
                        && collection.capabilities().support(Feature::ShowMenu)
                            == Support::Supported
                        && let Err(error) = handle.show_menu()
                    {
                        crate::error!(%error, "tray menu could not open");
                    }
                }
                if matches!(event.kind, EventKind::HostChanged(false)) {
                    crate::warn!(tray = %event.id, "tray host unavailable; registration is waiting");
                }
                if let EventKind::OperationFailed(message) = &event.kind {
                    crate::error!(tray = %event.id, %message, "native tray operation failed");
                }
                if matches!(event.kind, EventKind::Overflow) {
                    popup::dismiss_all(cx);
                    crate::error!("tray ingress saturated; input could not be delivered");
                }
                for handler in handlers.borrow_mut().iter_mut() {
                    handler(&event, cx);
                }
            });
        }
    });
    cx.set_global(Owner {
        host,
        dispatch,
        popups,
        animations: Default::default(),
        removal_quits: Default::default(),
        _startup: startup,
        _operations: operations,
        _events: events,
        _quit: quit,
    });

    Ok(services)
}

/// Finishes admitted native operations, including declarative startup registration.
/// # Errors
/// Reports a missing host; individual native failures remain on receipts/events.
pub fn flush(cx: &gpui::App) -> Request<()> {
    match cx.try_global::<Owner>() {
        Some(owner) => owner.host.flush(),
        None => Box::pin(async { Err(Error::HostUnavailable) }),
    }
}

/// Returns the application's weak tray collection from a foreground callback.
/// # Errors
/// Reports a host that has not been installed or has stopped.
pub fn collection(cx: &gpui::App) -> Result<Collection, Error> {
    let collection = cx
        .try_global::<Owner>()
        .ok_or(Error::HostUnavailable)?
        .host
        .collection();
    collection.items()?;
    Ok(collection)
}

/// Starts item-owned animation, replacing any previous playback for this ID.
/// Stops on hide/removal/host loss/shutdown or external icon updates. Reduced
/// motion applies the first frame and stops. No timer exists for a static item.
/// # Errors
/// Reports a missing/stopped host, absent item or unsupported image rendering.
pub fn animate(id: &str, animation: Animation, cx: &mut gpui::App) -> Result<(), Error> {
    let handle = cx
        .try_global::<Owner>()
        .ok_or(Error::HostUnavailable)?
        .host
        .collection()
        .get(id)?;
    let initial = handle.state()?;
    let mut revision = initial.requested_revision;
    let frames = animation.frames().to_vec();
    let interval = animation.interval();
    let id = id.to_owned();
    let task = cx.spawn(async move |cx| {
        let mut frame = 0;
        loop {
            let Ok(state) = handle.state() else {
                break;
            };
            if !state.requested_visible
                || state.registration != Registration::Registered
                || state.requested_revision != revision
            {
                break;
            }
            match handle.set_icon(frames[frame].clone()).await {
                Ok(applied) => revision = applied.revision,
                Err(error) => {
                    crate::error!(%error, "tray animation stopped");
                    break;
                }
            }
            if cx.update(|cx| cx.reduce_motion()) {
                break;
            }
            frame = (frame + 1) % frames.len();
            cx.background_executor().timer(interval).await;
        }
    });
    cx.global_mut::<Owner>().animations.insert(id, task);
    Ok(())
}
/// Cancels playback while preserving the last applied icon.
/// # Errors
/// Reports an absent/stopped tray host.
pub fn stop_animation(id: &str, cx: &mut gpui::App) -> Result<(), Error> {
    if cx.try_global::<Owner>().is_none() {
        return Err(Error::HostUnavailable);
    }
    cx.global_mut::<Owner>().animations.remove(id);
    Ok(())
}

#[cfg(all(test, feature = "view-test-support", not(target_family = "wasm")))]
pub(crate) struct StartupStopped;
#[cfg(all(test, feature = "view-test-support", not(target_family = "wasm")))]
impl gpui::Global for StartupStopped {}

#[cfg(all(test, feature = "view-test-support", not(target_family = "wasm")))]
pub(crate) fn install_for_tests(
    services: Services,
    specs: Vec<Tray>,
    handlers: Vec<Handler>,
    errors: StartupErrors,
    host: Result<rake_tray::Host, Error>,
    cx: &mut gpui::App,
) -> Result<Services, Error> {
    install_with(
        Installation {
            application: "tray.startup-test".into(),
            specs,
            handlers,
            popups: Vec::new(),
            errors,
        },
        services,
        move |_| host,
        |cx| {
            cx.set_global(StartupStopped);
            for window in cx.windows() {
                let _ = window.update(cx, |_, window, _| window.remove_window());
            }
        },
        cx,
    )
}

#[cfg(test)]
mod tests {
    //! Covers: native-menu command admission without wire retry-key exhaustion.
    //! Real: command registry, weak navigator, scheduler and service composition.
    //! Doubles: native presentation is absent.
    //! Requires: no graphical session.
    use super::*;
    use futures::{executor::LocalPool, task::LocalSpawnExt};
    use std::cell::Cell;

    #[crate::data(schema)]
    #[derive(crate::Command)]
    #[command(id = "tray.probe", capability = Headless)]
    struct Probe;

    #[test]
    fn removal_quit_restores_only_after_a_declined_guard() {
        let base = crate::App::new("dev.rake.removal-test").services().unwrap();
        let dispatch = Dispatcher::default();
        let registry = base.registry().with(dispatch.clone()).unwrap();
        let services =
            Services::with_platform(base.http(), base.diagnostics(), registry, base.platform());
        let runtime = crate::runtime::Runtime::new(Default::default(), services.clone());
        let blocked = Rc::new(Cell::new(true));
        let guard = blocked.clone();
        runtime
            .quit_guard(move |_| {
                let blocked = guard.get();
                async move {
                    if blocked {
                        Err(crate::router::GuardReason("keep item".into()))
                    } else {
                        Ok(())
                    }
                }
            })
            .unwrap();
        bind(runtime.router.navigator(), &services);
        let quit = dispatch.quit.borrow().clone().unwrap();
        let mut pool = LocalPool::new();
        let driver = runtime.router.clone();
        pool.spawner()
            .spawn_local(async move {
                driver.run().await.unwrap();
            })
            .unwrap();
        let restored = Rc::new(Cell::new(0));
        let restore = || {
            restored.set(restored.get() + 1);
            Box::pin(async {
                Ok(Applied {
                    revision: 1,
                    superseded: false,
                })
            }) as Request<Applied>
        };
        pool.run_until(quit_from_removal(Some(quit.clone()), restore))
            .unwrap();
        assert_eq!(
            restored.get(),
            1,
            "declined quit must restore the native item"
        );
        assert_eq!(runtime.quit_status().phase, crate::quit::QuitPhase::Blocked);
        blocked.set(false);
        pool.run_until(quit_from_removal(Some(quit), restore))
            .unwrap();
        assert_eq!(
            restored.get(),
            1,
            "accepted quit must not recreate native presentation"
        );
    }

    #[test]
    fn menu_commands_do_not_consume_wire_retry_keys_or_retain_the_runtime() {
        let base = crate::App::new("dev.rake.tray-test")
            .services()
            .expect("configured services");
        let dispatch = Dispatcher::default();
        let registry = base
            .registry()
            .with(dispatch.clone())
            .expect("install dispatch bridge");
        let services =
            Services::with_platform(base.http(), base.diagnostics(), registry, base.platform());
        let runtime =
            crate::runtime::Runtime::new(crate::router::RouterConfig::default(), services.clone());
        let count = Rc::new(Cell::new(0));
        let observed = count.clone();
        runtime
            .commands()
            .register(move |_: Probe, _| {
                let count = observed.clone();
                async move {
                    count.set(count.get() + 1);
                    Ok(())
                }
            })
            .expect("register command");
        bind(runtime.router.navigator(), &services);
        let run = dispatch
            .command
            .borrow()
            .clone()
            .expect("bound weak runtime");
        let mut pool = LocalPool::new();
        let driver = runtime.router.clone();
        pool.spawner()
            .spawn_local(async move {
                driver.run().await.expect("scheduler driver");
            })
            .expect("spawn local driver");

        for _ in 0..300 {
            run("tray.probe".into(), serde_json::Value::Null).expect("local activation admitted");
            pool.run_until_stalled();
        }

        assert_eq!(count.get(), 300);
        drop(runtime);
        assert!(matches!(
            run("tray.probe".into(), serde_json::Value::Null),
            Err(crate::control::ControlError::RuntimeShutdown)
        ));
    }
}
