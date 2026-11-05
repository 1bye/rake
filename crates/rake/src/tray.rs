//! Application-owned system tray icons, native menus and platform capabilities.
//!
//! Enable `tray`, declare items with [`crate::App::tray`], and access the shared
//! weak collection through [`crate::runtime::RuntimeContext::trays`]. Control
//! handles do not retain the runtime or unregister an item when dropped.

pub use rake_tray::{
    Anchor, Applied, Backend, Button, Capabilities, CheckState, Collection, CoordinateSpace, Error,
    Event, EventKind, Feature, Handle, HostPolicy, Image, ImageRepresentation, Input, Interaction,
    ItemKind, Menu, MenuItem, Point, Rect, Registration, Rendering, Request, State, Support, Tray,
};

use crate::application::startup::StartupErrors;
use crate::{AppError, Services};

struct Owner {
    host: rake_tray::Host,
    _startup: gpui::Task<()>,
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
    errors: StartupErrors,
    cx: &mut gpui::App,
) -> Result<Services, Error> {
    #[cfg(target_family = "wasm")]
    if specs.is_empty() {
        return Ok(services);
    }

    let host = rake_tray::Host::new(application)?;
    let collection = host.collection();
    let registry = services
        .registry()
        .with(collection.clone())
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
    cx.set_global(Owner {
        host,
        _startup: startup,
        _quit: quit,
    });

    Ok(services)
}
