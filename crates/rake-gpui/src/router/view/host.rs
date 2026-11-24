//! GPUI window registration and native lifecycle callbacks.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use gpui::{App, AsyncWindowContext, Window};

use crate::reload::host::ReloadUi;
use crate::tasks::{HostVisibility, host::HostObserver};

use crate::router::OperationError;
use rake_core::router::HostBinding;

use super::{Route, Router, WindowId};

#[cfg(all(feature = "native-lifecycle", target_os = "macos"))]
mod quit;
#[cfg(all(feature = "updates", not(target_family = "wasm")))]
mod updates;

pub(super) type WindowContexts = Rc<RefCell<BTreeMap<WindowId, AsyncWindowContext>>>;

#[derive(Default)]
struct GpuiHost {
    contexts: Option<WindowContexts>,
    observers: RefCell<BTreeMap<WindowId, HostObserver>>,
    reload: ReloadUi,
    #[cfg(all(feature = "native-lifecycle", target_os = "macos"))]
    native_quit: RefCell<Option<Rc<rake_updates::NativeQuitBridge>>>,
    #[cfg(all(feature = "updates", not(target_family = "wasm")))]
    update_revision: std::cell::Cell<Option<u64>>,
}

impl HostBinding for GpuiHost {
    fn detach_window(&self, id: WindowId) {
        self.observers.borrow_mut().remove(&id);

        if let Some(contexts) = &self.contexts {
            contexts.borrow_mut().remove(&id);
        }
    }

    fn shutdown(&self) {
        self.observers.borrow_mut().clear();
    }

    fn begin_reload(&self, windows: Vec<WindowId>) {
        self.reload.begin(windows);
    }

    fn reload_ready(&self, visibility: &dyn Fn(WindowId) -> Option<HostVisibility>) -> bool {
        self.reload.ready(visibility)
    }
}

impl<R: Route> Router<R> {
    pub(super) fn bind_view_contexts(&self, contexts: WindowContexts) {
        self.bind_host(GpuiHost {
            contexts: Some(contexts),
            ..GpuiHost::default()
        });
    }

    #[cfg(all(test, feature = "view-test-support", not(target_family = "wasm")))]
    pub(in crate::router::view) fn bind_initial_window(
        &self,
        id: WindowId,
        contexts: WindowContexts,
    ) {
        self.set_initial_window(id);
        self.bind_view_contexts(contexts);
    }

    /// Registers the UI context of a window reserved by a custom [`WindowHost`](crate::router::WindowHost).
    /// Call from the host's `open` callback before it resolves. The router must have
    /// been created with [`Self::in_window`]; headless routers reject this method.
    /// The embedding shell owns redrawing and keeping the shared driver alive.
    pub fn attach_window(
        &self,
        id: WindowId,
        window: &Window,
        cx: &App,
    ) -> Result<(), OperationError> {
        self.check_window(id)?;

        let contexts = self
            .gpui_host()
            .and_then(|host| host.contexts.as_ref())
            .ok_or(OperationError::InvalidOptions)?;

        contexts.borrow_mut().insert(id, window.to_async(cx));
        self.observe_window(id, window, cx, false);

        Ok(())
    }

    fn gpui_host(&self) -> Option<&GpuiHost> {
        (self.host_binding()? as &dyn std::any::Any).downcast_ref()
    }

    pub(crate) fn observe_host(&self, window: &Window, cx: &App) {
        self.observe_window(self.navigator().window_id(), window, cx, false);
    }

    /// Observes a standalone root while allowing its native window to close
    /// without requesting application quit. The runtime remains available for
    /// tray/background work until its owning host explicitly shuts it down.
    pub(crate) fn observe_root_host(&self, window: &Window, cx: &App) {
        self.observe_window(self.navigator().window_id(), window, cx, true);
    }

    fn observe_window(&self, id: WindowId, window: &Window, cx: &App, detach_on_close: bool) {
        if self.host_binding().is_none() {
            self.bind_host(GpuiHost::default());
        }

        let host = self.gpui_host().expect("GPUI host binding was installed");

        self.set_host_attached(true);

        #[cfg(not(target_family = "wasm"))]
        {
            let closing = self.navigator();

            window.on_window_should_close(cx, move |window, cx| {
                crate::window_state::native::capture(window, cx);

                let Some(router) = closing.upgrade_router() else {
                    return true;
                };

                if detach_on_close {
                    router.detach_host(id);

                    return true;
                }

                router.request_host_close(id)
            });
        }

        #[cfg(target_family = "wasm")]
        let _ = detach_on_close;

        let reporting = self.navigator();
        let shutdown = reporting.clone();

        #[cfg(target_family = "wasm")]
        let unload = reporting.clone();

        let observer = HostObserver::attach(
            move |visibility| {
                if let Some(router) = reporting.upgrade_router() {
                    let _ = router.set_window_visibility(id, visibility);
                }
            },
            move || {
                if let Some(router) = shutdown.upgrade_router() {
                    router.shutdown();
                }
            },
            move || {
                #[cfg(target_family = "wasm")]
                return unload
                    .upgrade_router()
                    .is_some_and(|router| router.warn_before_unload());

                #[cfg(not(target_family = "wasm"))]
                false
            },
            window,
            cx,
        );

        host.observers.borrow_mut().insert(id, observer);
    }

    pub(crate) fn detach_host(&self, id: WindowId) {
        let _ = self.set_window_visibility(id, HostVisibility::Hidden);

        if let Some(host) = self.gpui_host() {
            host.detach_window(id);
            self.set_host_attached(!host.observers.borrow().is_empty());
        }
    }

    pub(crate) fn observe_reload_frame(&self, id: WindowId, window: &mut Window) {
        if let Some(host) = self.gpui_host() {
            host.reload.observe_frame(id, window, self.navigator());
        }
    }

    pub(crate) fn install_quit_action(&self, cx: &mut App) {
        if self.host_binding().is_none() {
            self.bind_host(GpuiHost::default());
        }

        #[cfg(all(feature = "native-lifecycle", target_os = "macos"))]
        self.install_native_quit();

        let navigator = self.navigator();

        cx.on_action(move |_: &crate::quit::RequestQuit, _| {
            if let Some(router) = navigator.upgrade_router() {
                drop(router.quit(crate::quit::QuitOptions::default()));
            }
        });
    }
}
