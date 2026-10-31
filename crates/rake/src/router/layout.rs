//! Persistent GPUI layout composition for a routed workspace.
//!
//! A layout is created once for each physical workspace host and receives the
//! host's typed router capabilities. The workspace still owns the router driver,
//! browser history and host lifecycle; a layout only composes presentation around
//! [`RouteOutlet`](super::RouteOutlet).

use std::rc::Rc;

use gpui::{AnyView, App, Window};

use crate::runtime::RuntimeContext;
use crate::services::Services;

use super::{MountError, Navigator, OperationError, Route, RouteOutlet, Router, WindowId};

/// Capabilities shared by one persistent routed layout and its route outlet.
///
/// The navigator is a weak typed command proxy, while [`Self::runtime`] exposes
/// weak application capabilities for work that should not retain the routing
/// runtime. A context is scoped to one physical host; native secondary windows
/// receive distinct contexts and outlets.
#[derive(Clone)]
pub struct LayoutContext<R: Route> {
    pub(crate) navigator: Navigator<R>,
    pub(crate) services: Services,
    pub(crate) window_id: WindowId,
}

impl<R: Route> LayoutContext<R> {
    /// Constructs a context for one workspace host.
    pub(crate) fn new(router: Router<R>, services: Services, window_id: WindowId) -> Self {
        Self {
            navigator: router.navigator().for_window(window_id),
            services,
            window_id,
        }
    }

    /// Upgrades the typed router while its workspace runtime is alive.
    /// Keep this strong handle temporary when the layout factory retains view handles.
    /// # Errors
    /// Returns `RuntimeShutdown` after the owning workspace has shut down or dropped.
    pub fn router(&self) -> Result<Router<R>, OperationError> {
        self.navigator.runtime()
    }

    /// Returns a navigator bound to this physical host.
    pub fn navigator(&self) -> Navigator<R> {
        self.navigator.clone()
    }

    /// Returns the shared standard and application services.
    pub fn services(&self) -> Services {
        self.services.clone()
    }

    /// Returns the physical host represented by this context.
    pub fn window_id(&self) -> WindowId {
        self.window_id
    }

    /// Creates the selected-entry outlet bound to this physical host.
    pub fn outlet(&self) -> RouteOutlet<R> {
        RouteOutlet::from_navigator(self.navigator(), self.window_id)
    }

    /// Returns weak application capabilities associated with this host.
    pub fn runtime(&self) -> RuntimeContext<R> {
        RuntimeContext::from_navigator(self.navigator(), self.services.clone())
    }
}

/// Type-erased factory used by the routed workspace to create one layout entity
/// per physical host. The application builder adapts typed `Entity<L>` factories
/// into this representation.
pub(crate) type LayoutFactory<R> =
    Rc<dyn Fn(LayoutContext<R>, &mut Window, &mut App) -> Result<AnyView, MountError> + 'static>;
