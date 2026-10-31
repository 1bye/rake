//! Portable application runtime ownership and weak capability contexts.
//!
//! [`Runtime`] owns a headless [`Router`] and its configured [`Services`].
//! [`RuntimeContext`] deliberately keeps only a weak navigator, so a view or
//! task may retain its capabilities without retaining the runtime itself.

use std::rc::Rc;

use futures::future::{Either, select};

use crate::control::{Commands, RuntimeClient};
use crate::quit::{QuitOptions, QuitPhase, QuitResult, QuitStatus};
use crate::router::{Navigator, NoRoute, Operation, OperationError, Route, Router, RouterConfig};
use crate::services::Services;
use crate::tasks::{AppHandle, TaskScope};

/// Owns one portable, headless application runtime.
///
/// The owner is intentionally not `Clone`. Cloning a [`RuntimeClient`] or a
/// [`RuntimeContext`] does not make another runtime owner, and dropping this
/// value shuts down admission and managed work while leaving weak contexts
/// safe to retain.
pub struct Runtime<R: Route = NoRoute> {
    pub(crate) router: Router<R>,
    pub(crate) services: Services,
}

impl Runtime<NoRoute> {
    /// Creates a headless runtime with the supplied application services.
    pub fn new(config: RouterConfig, services: Services) -> Self {
        let router = Router::<NoRoute>::with_mount(
            config,
            services.diagnostics(),
            Rc::new(|_, _, _| {
                Box::pin(async { Ok(Rc::new(()) as crate::router::runtime::Resource) })
            }),
        );
        router.client().set_services(services.registry());

        Self { router, services }
    }
}

impl<R: Route> Runtime<R> {
    /// Returns a weak capability context for this runtime.
    pub fn context(&self) -> RuntimeContext<R> {
        RuntimeContext::from_router(&self.router, self.services.clone())
    }

    /// Returns the local runtime client while the owner is alive.
    pub fn client(&self) -> RuntimeClient<R> {
        self.router.client()
    }

    /// Returns the shared command registry.
    pub fn commands(&self) -> Commands {
        self.router.client().commands()
    }

    /// Registers an application-wide guarded quit check.
    pub fn quit_guard<F, Fut>(&self, guard: F) -> Result<(), OperationError>
    where
        F: Fn(crate::quit::QuitGuardContext) -> Fut + 'static,
        Fut: std::future::Future<Output = Result<(), crate::router::GuardReason>> + 'static,
    {
        self.router.quit_guard(guard)
    }

    /// Requests the same guarded quit used by the managed application host.
    pub fn quit(&self, options: QuitOptions) -> Operation<QuitResult> {
        self.router.quit(options)
    }

    /// Returns the current guarded-quit lifecycle state.
    pub fn quit_status(&self) -> QuitStatus {
        self.router.quit_status()
    }

    /// Stops admission and cancels all managed work.
    pub fn shutdown(&self) {
        self.router.shutdown();
    }

    /// Drives this runtime until explicit shutdown or a committed guarded quit.
    ///
    /// A headless runtime has no native installer or host-exit authority. A
    /// committed quit therefore shuts down this scheduler and returns; update
    /// handoff/installation remains the responsibility of a managed host.
    pub async fn run(&self) -> Result<(), OperationError> {
        let driver = Box::pin(self.router.run());
        let runtime = self.router.clone();
        let ready = Box::pin(async move {
            loop {
                if runtime.quit_status().phase == QuitPhase::Ready {
                    runtime.shutdown();
                    return;
                }

                let revision = runtime.snapshot().revision;
                runtime.changed(revision).await;
            }
        });

        futures::pin_mut!(driver, ready);
        match select(driver, ready).await {
            Either::Left((result, _)) => result,
            Either::Right(((), driver)) => driver.await,
        }
    }

    #[cfg(feature = "updates")]
    /// Returns the installed update controller, if application policy supplied one.
    pub fn updates(&self) -> Option<Rc<rake_updates::UpdateController>> {
        self.router.updates()
    }

    #[cfg(feature = "updates")]
    /// Checks for an available update without downloading or restarting.
    pub fn check_for_updates(&self) -> Result<(), rake_updates::Error> {
        self.router.check_for_updates()
    }

    #[cfg(feature = "updates")]
    /// Starts the provider's verified update download.
    pub fn download_update(&self) -> Result<(), rake_updates::Error> {
        self.router.download_update()
    }

    #[cfg(feature = "updates")]
    /// Submits staged update restart through the guarded quit boundary.
    pub fn restart_for_update(
        &self,
        options: QuitOptions,
    ) -> Result<Operation<QuitResult>, rake_updates::Error> {
        self.router.restart_for_update(options)
    }
}

impl<R: Route> Drop for Runtime<R> {
    fn drop(&mut self) {
        self.router.shutdown();
    }
}

/// Weak application capabilities tied to one runtime and one navigation origin.
#[derive(Clone)]
pub struct RuntimeContext<R: Route = NoRoute> {
    navigator: Navigator<R>,
    services: Services,
}

impl<R: Route> RuntimeContext<R> {
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

    pub(crate) fn from_router(router: &Router<R>, services: Services) -> Self {
        Self::from_navigator(router.navigator(), services)
    }

    pub(crate) fn from_navigator(navigator: Navigator<R>, services: Services) -> Self {
        Self {
            navigator,
            services,
        }
    }

    fn router(&self) -> Result<Router<R>, OperationError> {
        self.navigator.runtime()
    }

    /// Returns the configured standard and application service handles.
    pub fn services(&self) -> Services {
        self.services.clone()
    }

    /// Returns application-owned task capabilities retaining this origin.
    pub fn app(&self) -> AppHandle {
        self.navigator.app()
    }

    /// Returns application-owned task capabilities retaining this origin.
    pub fn tasks(&self) -> TaskScope {
        self.app().tasks()
    }

    /// Returns a client while the runtime is live, or `RuntimeShutdown` after
    /// owner teardown. The returned client temporarily keeps the router core
    /// alive, but cannot reopen admission after shutdown.
    pub fn client(&self) -> Result<RuntimeClient<R>, OperationError> {
        Ok(self.router()?.client())
    }

    /// Requests guarded application quit. Dropped/closed contexts return a
    /// completed operation carrying `RuntimeShutdown`.
    pub fn quit(&self, options: QuitOptions) -> Operation<QuitResult> {
        self.navigator.quit(options)
    }

    /// Returns quit state while the runtime is alive.
    pub fn quit_status(&self) -> Result<QuitStatus, OperationError> {
        Ok(self.router()?.quit_status())
    }

    #[cfg(feature = "updates")]
    /// Returns the installed update controller while the runtime is alive.
    pub fn updates(&self) -> Result<Option<Rc<rake_updates::UpdateController>>, OperationError> {
        Ok(self.router()?.updates())
    }

    #[cfg(feature = "updates")]
    /// Checks for an available update while the runtime is alive.
    pub fn check_for_updates(&self) -> Result<(), rake_updates::Error> {
        self.router()
            .map_err(|error| rake_updates::Error::Policy(error.to_string()))?
            .check_for_updates()
    }

    #[cfg(feature = "updates")]
    /// Starts the provider's verified update download while the runtime is alive.
    pub fn download_update(&self) -> Result<(), rake_updates::Error> {
        self.router()
            .map_err(|error| rake_updates::Error::Policy(error.to_string()))?
            .download_update()
    }

    #[cfg(feature = "updates")]
    /// Submits staged update restart through the guarded quit boundary.
    pub fn restart_for_update(
        &self,
        options: QuitOptions,
    ) -> Result<Operation<QuitResult>, rake_updates::Error> {
        self.router()
            .map_err(|error| rake_updates::Error::Policy(error.to_string()))?
            .restart_for_update(options)
    }
}
