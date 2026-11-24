//! Mount callbacks for the GPUI test platform, which has no native host.

use rake_core::router::{RouteMount, RouteMountContext};
use std::any::Any;
use std::rc::Rc;

use futures::future::LocalBoxFuture;

use super::{
    EntryHandle, MountError, MountedRoute, Navigator, PreparedPage, Route, Router, RouterConfig,
};
use crate::diagnostics::Collector;
use crate::http::CancellationToken;
use rake_core::services::ServiceRegistry;

pub(crate) type Resource = Rc<dyn Any>;

pub(crate) type Mount<R> = Rc<
    dyn Fn(
        R,
        EntryHandle<R>,
        Navigator<R>,
    ) -> LocalBoxFuture<'static, Result<Resource, MountError>>,
>;

pub(crate) type Prepare<R> =
    Rc<dyn Fn(R, CancellationToken) -> LocalBoxFuture<'static, Result<PreparedPage, MountError>>>;

pub(crate) type PreparedMount<R> = Rc<
    dyn Fn(
        R,
        EntryHandle<R>,
        Navigator<R>,
        PreparedPage,
    ) -> LocalBoxFuture<'static, Result<Resource, MountError>>,
>;

pub(super) struct TestResource(pub Resource);
impl MountedRoute for TestResource {}

struct TestMount<R: Route> {
    prepare: Prepare<R>,
    mount: PreparedMount<R>,
}

impl<R: Route> RouteMount<R> for TestMount<R> {
    fn prepare(
        &self,
        destination: &R,
        _services: ServiceRegistry,
        cancellation: CancellationToken,
    ) -> LocalBoxFuture<'static, Result<PreparedPage, MountError>> {
        // Existing fixtures prepare only from destination and cancellation.
        (self.prepare)(destination.clone(), cancellation)
    }

    fn mount(
        &self,
        destination: &R,
        prepared: PreparedPage,
        context: RouteMountContext<R>,
    ) -> LocalBoxFuture<'static, Result<Box<dyn MountedRoute>, MountError>> {
        let mount = (self.mount)(
            destination.clone(),
            context.entry,
            Navigator::from_core(context.navigator),
            prepared,
        );
        Box::pin(async move { Ok(Box::new(TestResource(mount.await?)) as Box<dyn MountedRoute>) })
    }
}

impl<R: Route> Router<R> {
    pub(crate) fn with_mount(config: RouterConfig, collector: Collector, mount: Mount<R>) -> Self {
        let prepared_mount: PreparedMount<R> =
            Rc::new(move |destination, entry, navigator, _| mount(destination, entry, navigator));

        let prepare: Prepare<R> = Rc::new(|_, _| Box::pin(async { Ok(PreparedPage::empty()) }));

        Self::with_prepared_mount(config, collector, prepare, prepared_mount)
    }

    pub(crate) fn with_prepared_mount(
        config: RouterConfig,
        collector: Collector,
        prepare: Prepare<R>,
        mount: PreparedMount<R>,
    ) -> Self {
        let adapter = TestMount { prepare, mount };

        Self::from_core(rake_core::router::Router::with_route_mount_and_collector(
            config,
            Default::default(),
            collector,
            adapter,
        ))
    }
}
