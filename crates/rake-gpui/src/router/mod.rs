//! Typed routing with GPUI-backed route views and window lifecycle.

use crate::services::ServiceRegistry;
use std::ops::Deref;

pub use layout::LayoutContext;
pub use navigation_view::NavigationBoundary;

#[doc(hidden)]
pub use page::{
    __mount_loaded_page, __mount_page, __prepare_loaded_page, __routes, __routes_prepared,
};

pub use page::{LoadedPage, Page};
pub use rake_core::router::*;
pub use view::{PageContext, RouteOutlet, RouteViews, TabBar};

/// Owned input to a route admission policy, including the application's services.
pub type AdmissionContext<R> = rake_core::router::AdmissionContext<R, ServiceRegistry>;

#[doc(hidden)]
pub type AdmissionCallback<R> = std::rc::Rc<
    dyn Fn(AdmissionContext<R>) -> futures::future::LocalBoxFuture<'static, Admission<R>>,
>;

pub(crate) mod layout;
mod mount;
mod navigation_view;
mod page;
mod portable;

pub use mount::{RouteMount, RouteMountContext};

#[cfg(all(test, feature = "view-test-support", not(target_family = "wasm")))]
pub(crate) mod test_mount;
pub(crate) mod view;
#[cfg(target_family = "wasm")]
pub(crate) mod web;

/// Owns a typed router and exposes GPUI host attachment.
#[derive(Clone)]
pub struct Router<R: Route>(rake_core::router::Router<R>);

impl<R: Route> Router<R> {
    /// Creates a headless routing runtime.
    pub fn new(config: RouterConfig) -> Self {
        Self(rake_core::router::Router::new(config))
    }

    /// Creates a headless routing runtime with the supplied services.
    pub fn with_services(config: RouterConfig, services: ServiceRegistry) -> Self {
        Self(rake_core::router::Router::with_services(
            config,
            services.into_core(),
        ))
    }

    /// Creates a routing runtime with a retained mount adapter.
    pub fn with_route_mount(
        config: RouterConfig,
        services: ServiceRegistry,
        adapter: impl RouteMount<R>,
    ) -> Self {
        Self(rake_core::router::Router::with_route_mount(
            config,
            services.into_core(),
            mount::FacadeMount(adapter),
        ))
    }

    pub(crate) fn from_core(core: rake_core::router::Router<R>) -> Self {
        Self(core)
    }

    /// Returns a local control client with the application's GPUI services.
    pub fn client(&self) -> crate::control::RuntimeClient<R> {
        crate::control::RuntimeClient::from_core(self.0.client())
    }

    /// Returns a weak navigation handle for this workspace.
    pub fn navigator(&self) -> Navigator<R> {
        Navigator(self.0.navigator())
    }
}

impl<R: Route> Deref for Router<R> {
    type Target = rake_core::router::Router<R>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Weak, origin-scoped navigation and GPUI host access.
#[derive(Clone)]
pub struct Navigator<R: Route>(rake_core::router::Navigator<R>);

impl<R: Route> Navigator<R> {
    pub(crate) fn from_core(core: rake_core::router::Navigator<R>) -> Self {
        Self(core)
    }

    pub(crate) fn into_core(self) -> rake_core::router::Navigator<R> {
        self.0
    }

    pub(crate) fn upgrade_router(&self) -> Option<Router<R>> {
        self.0.upgrade_router().map(Router::from_core)
    }

    pub(crate) fn runtime(&self) -> Result<Router<R>, OperationError> {
        self.0.runtime().map(Router::from_core)
    }

    /// Retargets this navigator to a physical window.
    pub fn for_window(&self, window: WindowId) -> Self {
        Self(self.0.for_window(window))
    }

    /// Upgrades this navigator for use in a custom native window host.
    #[cfg(not(target_family = "wasm"))]
    pub fn router(&self) -> Option<Router<R>> {
        self.upgrade_router()
    }
}

impl<R: Route> Deref for Navigator<R> {
    type Target = rake_core::router::Navigator<R>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
