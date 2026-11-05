//! Application-owned system tray icons, native menus and platform capabilities.
//!
//! Enable `tray`, declare items with [`crate::App::tray`], and access the shared
//! weak collection through [`crate::runtime::RuntimeContext::trays`]. Control
//! handles do not retain the runtime or unregister an item when dropped.

pub use rake_tray::{
    Accelerator, Anchor, Applied, Backend, Button, Capabilities, CheckState, Collection,
    CoordinateSpace, Error, Event, EventKind, Feature, Handle, HostPolicy, Image,
    ImageRepresentation, Input, Interaction, ItemKind, Menu, MenuItem, Point, Rect, Registration,
    Rendering, Request, State, Support, Tray, linux, macos, windows,
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

#[derive(Clone, Default)]
struct Dispatcher(Rc<RefCell<Option<Dispatch>>>);

pub(crate) fn bind<R: crate::router::Route>(
    navigator: crate::router::Navigator<R>,
    services: &Services,
) {
    if let Ok(dispatcher) = services.get::<Dispatcher>() {
        *dispatcher.0.borrow_mut() = Some(Rc::new(move |id, arguments| {
            let router = navigator
                .runtime()
                .map_err(|_| crate::control::ControlError::RuntimeShutdown)?;
            router.client().dispatch_tray_command(id, arguments)
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
        let dispatch = dispatcher.0.borrow().clone();
        let result = dispatch
            .ok_or(crate::control::ControlError::RuntimeShutdown)
            .and_then(|dispatch| dispatch(command_id.clone(), arguments.clone()));
        if let Err(error) = result {
            crate::error!(%error, "tray command rejected");
        }
    }))
}

struct Owner {
    host: rake_tray::Host,
    dispatch: Dispatcher,
    _startup: gpui::Task<()>,
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
    errors: StartupErrors,
    cx: &mut gpui::App,
) -> Result<Services, Error> {
    #[cfg(target_family = "wasm")]
    if specs.is_empty() {
        return Ok(services);
    }

    let host = rake_tray::Host::new(application)?;
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
    let requests = specs
        .into_iter()
        .map(|spec| collection.create(spec))
        .collect::<Vec<_>>();
    let startup = cx.spawn(async move |cx| {
        for request in requests {
            if let Err(error) = request.await {
                cx.update(|cx| {
                    errors.record(AppError::Tray(error));
                    stop(cx);
                });

                return;
            }
        }
    });
    let quit = cx.on_app_quit(|cx| {
        let shutdown = cx.global::<Owner>().host.shutdown();
        async move {
            if let Err(error) = shutdown.await {
                crate::error!(%error, "native tray shutdown failed");
            }
        }
    });
    let events = cx.spawn(async move |cx| {
        let mut handlers = handlers;
        while let Ok(event) = collection.next_event().await {
            cx.update(|cx| {
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
                        cx.dispatch_action(&crate::quit::RequestQuit);
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
                if matches!(event.kind, EventKind::Overflow) {
                    crate::error!("tray ingress saturated; input could not be delivered");
                }
                for handler in &mut handlers {
                    handler(&event, cx);
                }
            });
        }
    });
    cx.set_global(Owner {
        host,
        dispatch,
        _startup: startup,
        _events: events,
        _quit: quit,
    });

    Ok(services)
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
        let run = dispatch.0.borrow().clone().expect("bound weak runtime");
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
