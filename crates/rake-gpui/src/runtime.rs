//! GPUI bindings for the application runtime and its weak context.

use crate::quit::{QuitOptions, QuitResult, QuitStatus};
use crate::router::{Navigator, NoRoute, Route, Router, RouterConfig};
use crate::router::{Operation, OperationError};
use crate::services::Services;
use crate::tasks::{AppHandle, TaskScope};
#[cfg(feature = "updates")]
use std::rc::Rc;

/// Owns a framework-free runtime and exposes GPUI binding contexts.
pub struct Runtime<R: Route = NoRoute> {
    core: rake_core::runtime::Runtime<R>,
    pub(crate) router: Router<R>,
}

impl Runtime<NoRoute> {
    /// Creates a runtime with the supplied application services.
    pub fn new(config: RouterConfig, services: Services) -> Self {
        let core = rake_core::runtime::Runtime::new(config, services.into_core());
        let router = Router::from_core(core.router().clone());

        Self { core, router }
    }
}

impl<R: Route> Runtime<R> {
    /// Returns this runtime's local control client.
    pub fn client(&self) -> crate::control::RuntimeClient<R> {
        self.router.client()
    }

    /// Returns the shared command registry.
    pub fn commands(&self) -> crate::control::Commands {
        self.client().commands()
    }

    /// Returns a weak capability context with GPUI view bindings.
    pub fn context(&self) -> RuntimeContext<R> {
        RuntimeContext {
            core: self.core.context(),
        }
    }
}

/// Weak application capabilities and GPUI loading/action bindings.
#[derive(Clone)]
pub struct RuntimeContext<R: Route = NoRoute> {
    core: rake_core::runtime::RuntimeContext<R>,
}

impl<R: Route> RuntimeContext<R> {
    /// Returns a client while the runtime is live.
    pub fn client(
        &self,
    ) -> Result<crate::control::RuntimeClient<R>, crate::router::OperationError> {
        self.core
            .client()
            .map(crate::control::RuntimeClient::from_core)
    }

    pub(crate) fn from_navigator(navigator: Navigator<R>, services: Services) -> Self {
        Self {
            core: rake_core::runtime::RuntimeContext::from_navigator(
                navigator.into_core(),
                services.into_core(),
            ),
        }
    }

    /// Returns the configured standard and application service handles.
    pub fn services(&self) -> Services {
        Services::from_core(self.core.services())
    }

    /// Returns weak access to the native application tray host.
    /// # Errors
    /// Reports an unavailable GUI host or stopped runtime.
    #[cfg(feature = "tray")]
    pub fn trays(&self) -> Result<crate::tray::Collection, crate::tray::Error> {
        self.client().map_err(|_| crate::tray::Error::Stopped)?;
        #[cfg(target_family = "wasm")]
        {
            Err(crate::tray::Error::Unsupported(
                crate::tray::Feature::MultipleItems,
            ))
        }

        #[cfg(not(target_family = "wasm"))]
        {
            self.services()
                .get::<crate::tray::Collection>()
                .map(|c| c.as_ref().clone())
                .map_err(|_| crate::tray::Error::HostUnavailable)
        }
    }

    /// Binds an initiated user-action future to this view, with automatic redraw,
    /// cancellation and finite-work quit tracking. Start gesture-sensitive browser
    /// requests before calling this method. For reusable preparation, use `load`.
    #[cfg(feature = "gpui")]
    pub fn action<V: 'static, T: 'static, E, Fut>(
        &self,
        cx: &mut gpui::Context<V>,
        future: Fut,
    ) -> crate::loading::AsyncAction<T, E>
    where
        E: std::error::Error + 'static,
        Fut: std::future::Future<Output = Result<T, E>> + 'static,
    {
        crate::loading::AsyncAction::new(Ok(self.tasks()), self.services(), future, cx)
    }

    /// Observes the shared update controller and redraws this view automatically.
    /// Dropping the handle/view stops observation, without stopping provider work.
    /// Missing policy and action failures are represented in the returned handle.
    #[cfg(all(feature = "gpui", feature = "updates"))]
    pub fn bind_updates<V: 'static>(&self, cx: &mut gpui::Context<V>) -> crate::UpdateHandle {
        crate::updates_binding::bind_updates(self.clone(), cx)
    }

    #[cfg(all(feature = "gpui", feature = "updates"))]
    pub(crate) async fn stopped(&self) {
        self.core.stopped().await;
    }

    /// Starts a view-owned asynchronous value and automatically redraws its view.
    /// Work survives hidden views and is cancelled when the loader is dropped.
    /// Admission and source failures are exposed by the returned handle.
    #[cfg(feature = "gpui")]
    pub fn load<V: 'static, T: 'static, E, F, Fut>(
        &self,
        cx: &mut gpui::Context<V>,
        factory: F,
    ) -> crate::loading::Loader<T, E>
    where
        E: std::error::Error + 'static,
        F: Fn(crate::loading::LoadContext) -> Fut + 'static,
        Fut: std::future::Future<Output = Result<T, E>> + 'static,
    {
        crate::loading::Loader::new(Ok(self.tasks()), self.services(), factory, cx)
    }

    /// Reveals the application's registered repeatable standalone main window.
    /// # Errors
    /// Rejects a stopped runtime, missing factory, browser host or failed mount.
    #[cfg(feature = "gpui")]
    pub fn show_main_window(&self, cx: &mut gpui::App) -> Result<(), crate::AppError> {
        self.client()
            .map_err(|error| crate::AppError::Window(error.to_string()))?;

        crate::show_main_window(cx)
    }
}

impl<R: Route> Runtime<R> {
    /// Registers an application-wide guarded quit check.
    pub fn quit_guard<F, Fut>(&self, guard: F) -> Result<(), OperationError>
    where
        F: Fn(crate::quit::QuitGuardContext) -> Fut + 'static,
        Fut: std::future::Future<Output = Result<(), crate::router::GuardReason>> + 'static,
    {
        self.core.quit_guard(guard)
    }

    /// Requests the same guarded quit used by the managed application host.
    pub fn quit(&self, options: QuitOptions) -> Operation<QuitResult> {
        self.core.quit(options)
    }

    /// Returns the current guarded-quit lifecycle state.
    pub fn quit_status(&self) -> QuitStatus {
        self.core.quit_status()
    }

    /// Stops admission and cancels all managed work.
    pub fn shutdown(&self) {
        self.core.shutdown()
    }

    /// Drives this runtime until explicit shutdown or a committed guarded quit.
    ///
    /// A headless runtime has no native installer or host-exit authority. A
    /// committed quit therefore shuts down this scheduler and returns; update
    /// handoff/installation remains the responsibility of a managed host.
    pub async fn run(&self) -> Result<(), OperationError> {
        self.core.run().await
    }

    #[cfg(feature = "updates")]
    /// Returns the installed update controller, if application policy supplied one.
    pub fn updates(&self) -> Option<Rc<rake_updates::UpdateController>> {
        self.core.updates()
    }

    #[cfg(feature = "updates")]
    /// Checks for an available update without downloading or restarting.
    pub fn check_for_updates(&self) -> Result<(), rake_updates::Error> {
        self.core.check_for_updates()
    }

    #[cfg(feature = "updates")]
    /// Starts the provider's verified update download.
    pub fn download_update(&self) -> Result<(), rake_updates::Error> {
        self.core.download_update()
    }

    #[cfg(feature = "updates")]
    /// Submits staged update restart through the guarded quit boundary.
    pub fn restart_for_update(
        &self,
        options: QuitOptions,
    ) -> Result<Operation<QuitResult>, rake_updates::Error> {
        self.core.restart_for_update(options)
    }
}

impl<R: Route> RuntimeContext<R> {
    /// Returns application-owned task capabilities retaining this origin.
    pub fn app(&self) -> AppHandle {
        self.core.app()
    }

    /// Returns application-owned task capabilities retaining this origin.
    pub fn tasks(&self) -> TaskScope {
        self.core.tasks()
    }

    /// Requests guarded application quit. Dropped/closed contexts return a
    /// completed operation carrying `RuntimeShutdown`.
    pub fn quit(&self, options: QuitOptions) -> Operation<QuitResult> {
        self.core.quit(options)
    }

    /// Returns quit state while the runtime is alive.
    pub fn quit_status(&self) -> Result<QuitStatus, OperationError> {
        self.core.quit_status()
    }

    /// Rechecks a changed browser deployment and submits guarded application reload.
    /// The runtime performs reload only after retained-entry/application guards and
    /// finite-work policy approve. Cancellation/denial keeps the running app intact.
    /// Observe this outside a finite managed task, so quit does not wait on itself.
    /// The returned operation may be dropped without cancelling the reload.
    ///
    /// # Errors
    /// Rejects native hosts, unchanged/unreachable deployments and stopped runtimes.
    pub async fn reload_web_update(
        &self,
        deployment: &crate::web::Deployment,
        options: QuitOptions,
    ) -> Result<Operation<QuitResult>, crate::web::Error> {
        self.core.reload_web_update(deployment, options).await
    }

    #[cfg(feature = "updates")]
    /// Returns the installed update controller while the runtime is alive.
    pub fn updates(&self) -> Result<Option<Rc<rake_updates::UpdateController>>, OperationError> {
        self.core.updates()
    }

    #[cfg(feature = "updates")]
    /// Checks for an available update while the runtime is alive.
    pub fn check_for_updates(&self) -> Result<(), rake_updates::Error> {
        self.core.check_for_updates()
    }

    #[cfg(feature = "updates")]
    /// Starts the provider's verified update download while the runtime is alive.
    pub fn download_update(&self) -> Result<(), rake_updates::Error> {
        self.core.download_update()
    }

    #[cfg(feature = "updates")]
    /// Submits staged update restart through the guarded quit boundary.
    pub fn restart_for_update(
        &self,
        options: QuitOptions,
    ) -> Result<Operation<QuitResult>, rake_updates::Error> {
        self.core.restart_for_update(options)
    }
}
