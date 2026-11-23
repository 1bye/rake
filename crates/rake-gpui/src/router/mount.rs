//! Route mount entry points with the GPUI facade's service and navigation handles.

use super::{
    EntryHandle, EntryId, MountError, MountedRoute, Navigator, PreparedPage, Route, WindowId,
};

use crate::http::CancellationToken;
use crate::services::ServiceRegistry;
use futures::future::LocalBoxFuture;

/// The provisional entry and its host, supplied when mounting a fresh route.
pub struct RouteMountContext<R: Route> {
    /// Window that owns the entry. A secondary host has already been opened.
    pub window_id: WindowId,
    /// Stable identity of this visit, preserved across history and checkpoint restore.
    pub entry_id: EntryId,
    /// Weak handle valid during mount, including for reading restored entry state.
    pub entry: EntryHandle<R>,
    /// Weak navigator bound to the entry's window and tab.
    pub navigator: Navigator<R>,
}

/// Prepares and mounts typed destinations on the caller-driven router loop.
///
/// Both callbacks can use local futures. The router owns admission and history,
/// and publishes the mount only after the transition commits.
pub trait RouteMount<R: Route>: 'static {
    /// Prepares owned data after admission and initial guards, before mounting
    /// or opening a secondary host. During restore, all entries prepare before
    /// any secondary host is opened.
    ///
    /// The router can drop the future on cancellation. Use the token for
    /// cooperative work, and release provisional resources on drop.
    ///
    /// # Errors
    /// Return a preparation error to leave the current workspace intact.
    fn prepare(
        &self,
        _destination: &R,
        _services: ServiceRegistry,
        _cancellation: CancellationToken,
    ) -> LocalBoxFuture<'static, Result<PreparedPage, MountError>> {
        Box::pin(async { Ok(PreparedPage::empty()) })
    }

    /// Consumes prepared data and creates a provisional mount.
    ///
    /// Keep it hidden until committed presentation. During restore, read
    /// entry checkpoints through the weak handle before returning. On error or
    /// cancellation before a handle is returned, the adapter must clean up
    /// resources it has created. After it returns a handle, the router disposes
    /// it on rollback, removal, close or shutdown.
    ///
    /// # Errors
    /// Return a mount error to leave the current workspace intact.
    fn mount(
        &self,
        destination: &R,
        prepared: PreparedPage,
        context: RouteMountContext<R>,
    ) -> LocalBoxFuture<'static, Result<Box<dyn MountedRoute>, MountError>>;
}

pub(super) struct FacadeMount<A>(pub A);

impl<R: Route, A: RouteMount<R>> rake_core::router::RouteMount<R> for FacadeMount<A> {
    fn prepare(
        &self,
        destination: &R,
        services: rake_core::ServiceRegistry,
        cancellation: CancellationToken,
    ) -> LocalBoxFuture<'static, Result<PreparedPage, MountError>> {
        self.0.prepare(
            destination,
            ServiceRegistry::from_core(services),
            cancellation,
        )
    }

    fn mount(
        &self,
        destination: &R,
        prepared: PreparedPage,
        context: rake_core::router::RouteMountContext<R>,
    ) -> LocalBoxFuture<'static, Result<Box<dyn MountedRoute>, MountError>> {
        self.0.mount(
            destination,
            prepared,
            RouteMountContext {
                window_id: context.window_id,
                entry_id: context.entry_id,
                entry: context.entry,
                navigator: Navigator::from_core(context.navigator),
            },
        )
    }
}
