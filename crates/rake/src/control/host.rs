//! Route-independent dispatch shared by the native and Web development transports.

use std::rc::Rc;

use super::{ControlError, Reply, Request, RuntimeClient, RuntimeInfo};
use crate::router::Route;

pub(super) trait Host {
    fn info(&self) -> RuntimeInfo;
    fn request(&self, request: Request) -> Result<Reply, ControlError>;

    #[cfg(any(unix, target_family = "wasm"))]
    fn begin_response(&self) -> Option<super::ResponseGuard> {
        None
    }

    #[cfg(target_family = "wasm")]
    fn reload_status(&self) -> Option<crate::reload::ReloadStatus> {
        None
    }

    #[cfg(target_family = "wasm")]
    fn reload<'a>(
        &'a self,
        bridge: &'a super::WebBridge,
        application_id: &'a str,
        build: u64,
    ) -> futures::future::LocalBoxFuture<'a, Result<(), String>>;
}

#[derive(Clone)]
pub(crate) struct HostClient(Rc<dyn Host>);

impl HostClient {
    pub(super) fn new(client: impl Host + 'static) -> Self {
        Self(Rc::new(client))
    }
    pub(crate) fn info(&self) -> RuntimeInfo {
        self.0.info()
    }
    pub(crate) fn request(&self, request: Request) -> Result<Reply, ControlError> {
        self.0.request(request)
    }

    #[cfg(any(unix, target_family = "wasm"))]
    pub(super) fn begin_response(&self) -> Option<super::ResponseGuard> {
        self.0.begin_response()
    }

    #[cfg(target_family = "wasm")]
    pub(super) fn reload_status(&self) -> Option<crate::reload::ReloadStatus> {
        self.0.reload_status()
    }

    #[cfg(target_family = "wasm")]
    pub(super) async fn reload(
        &self,
        bridge: &super::WebBridge,
        application_id: &str,
        build: u64,
    ) -> Result<(), String> {
        self.0.reload(bridge, application_id, build).await
    }
}

impl<R: Route> Host for RuntimeClient<R> {
    fn info(&self) -> RuntimeInfo {
        self.info()
    }
    fn request(&self, request: Request) -> Result<Reply, ControlError> {
        self.request(request)
    }

    #[cfg(any(unix, target_family = "wasm"))]
    fn begin_response(&self) -> Option<super::ResponseGuard> {
        Some(self.begin_response())
    }

    #[cfg(target_family = "wasm")]
    fn reload_status(&self) -> Option<crate::reload::ReloadStatus> {
        Some(self.router().reload_status())
    }

    #[cfg(target_family = "wasm")]
    fn reload<'a>(
        &'a self,
        bridge: &'a super::WebBridge,
        application_id: &'a str,
        build: u64,
    ) -> futures::future::LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(bridge.reload_runtime(self, application_id, build))
    }
}
