//! Application configuration and the native/browser GPUI bootstrap boundary.

#[cfg(not(target_family = "wasm"))]
use std::sync::Arc;

use gpui::{AnyView, AppContext, Context, IntoElement, Render, Window, WindowOptions};

use crate::diagnostics::{DiagnosticsConfig, DiagnosticsError};
use crate::http::{HttpClient, HttpConfig};
use crate::router::runtime::{AdmissionCallback, Guard};
use crate::services::{ServiceError, ServiceRegistry, Services};

use crate::router::{
    GuardContext, GuardResult, NoRoute, Route, RouteViews, RouterConfig, WebConfig,
};

use std::{cell::RefCell, rc::Rc};

type Root = Box<
    dyn FnOnce(
        crate::runtime::RuntimeContext,
        &mut Window,
        &mut gpui::App,
    ) -> Result<AnyView, crate::router::MountError>,
>;
type WindowRoot = Box<
    dyn FnOnce(Services, &mut Window, &mut gpui::App) -> Result<AnyView, crate::router::MountError>,
>;
type Background = Box<dyn FnOnce(crate::runtime::RuntimeContext, &mut gpui::App)>;

/// What closing a standalone application's window requests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ClosePolicy {
    /// Run application quit guards and stop managed work before exiting.
    #[default]
    Quit,
    /// Close the window while keeping the application's background runtime alive.
    KeepRunning,
}

mod factory;
mod managed;
#[doc(hidden)]
pub use factory::{AppFactoryOutput, run_app_factory};
mod startup;
#[cfg(all(feature = "updates", not(target_family = "wasm")))]
mod update_services;

#[cfg(feature = "experimental-view")]
type ThemeInstaller = Box<dyn FnOnce(&mut gpui::App)>;

#[cfg(feature = "experimental-view")]
fn install_themes(themes: Vec<ThemeInstaller>, cx: &mut gpui::App) {
    for install in themes {
        install(cx);
    }
}

#[cfg(target_family = "wasm")]
thread_local! {
    // The browser owns the event loop; retaining the handle owns GPUI until page disposal.
    static BROWSER_APP: std::cell::RefCell<Option<gpui::ApplicationHandle>> = const {
        std::cell::RefCell::new(None)
    };
}

/// Portable application configuration for a root view or a retained typed router.
/// Typed routers drive scoped tasks and entry activity on the host UI loop.
pub struct App<R: Route = NoRoute> {
    id: String,
    services: ServiceRegistry,
    window_options: WindowOptions,
    workspace_chrome: bool,
    navigation_feedback: bool,
    http: HttpConfig,
    http_client: RefCell<Option<HttpClient>>,
    layout: Option<crate::router::layout::LayoutFactory<R>>,
    new_window_options: Option<Rc<dyn Fn(crate::router::WindowId) -> WindowOptions>>,
    configuration_error: Option<&'static str>,
    diagnostics: DiagnosticsConfig,
    root: Option<Root>,
    background: Option<Background>,
    close_policy: ClosePolicy,
    close_policy_set: bool,
    route_options: bool,
    quit_guards: Vec<crate::router::runtime::QuitGuard>,
    routes: Option<Rc<dyn RouteViews<R>>>,
    initial: Option<R>,
    router: RouterConfig,
    web: WebConfig,
    guards: Vec<Guard<R>>,
    admissions: Vec<AdmissionCallback<R>>,
    control: crate::control::ControlConfig,
    commands: crate::control::Commands,
    #[cfg(feature = "experimental-view")]
    themes: Vec<ThemeInstaller>,
    #[cfg(feature = "updates")]
    distribution: Option<(&'static str, &'static str)>,
}

impl App<NoRoute> {
    /// Creates an application. Its nonempty, whitespace-free ID is validated at startup.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            services: ServiceRegistry::new(),
            window_options: WindowOptions::default(),
            workspace_chrome: true,
            navigation_feedback: true,
            http: HttpConfig::new(),
            http_client: RefCell::new(None),
            layout: None,
            new_window_options: None,
            configuration_error: None,
            diagnostics: DiagnosticsConfig::new(),
            root: None,
            background: None,
            close_policy: ClosePolicy::default(),
            close_policy_set: false,
            route_options: false,
            quit_guards: Vec::new(),
            routes: None,
            initial: None,
            router: RouterConfig::default(),
            web: WebConfig::new(),
            guards: Vec::new(),
            admissions: Vec::new(),
            control: crate::control::ControlConfig::default(),
            commands: crate::control::Commands::new(),
            #[cfg(feature = "experimental-view")]
            themes: Vec::new(),
            #[cfg(feature = "updates")]
            distribution: None,
        }
    }
    /// Registers the application's typed screens once. Defaults to the `/` route.
    ///
    /// Configure `initial`, `entry_guard`, and `entry_admission` after this type
    /// transition. To change factories for the same destination type, use
    /// [`App::replace_routes`], which preserves the initial destination, guards,
    /// and admissions.
    ///
    /// ```compile_fail
    /// #[derive(Clone, rake::Route)]
    /// enum Destination { #[route(id = "home", path = "/")] Home }
    /// let pages = || rake::router::__routes(|_: &Destination, _, _, _| {
    ///     Err(rake::router::MountError("fixture".into()))
    /// });
    /// let app = rake::App::new("example.routes").routes(pages());
    /// let app = app.routes(pages); // use replace_routes instead
    /// ```
    pub fn routes<Next: Route>(self, routes: impl RouteViews<Next>) -> App<Next> {
        let configuration_error = if self.layout.is_some()
            || !self.guards.is_empty()
            || !self.admissions.is_empty()
        {
            Some("register routes before configuring layouts, entry guards, or entry admissions")
        } else {
            self.configuration_error
        };
        App {
            id: self.id,
            services: self.services,
            window_options: self.window_options,
            workspace_chrome: self.workspace_chrome,
            navigation_feedback: self.navigation_feedback,
            http: self.http,
            http_client: self.http_client,
            layout: None,
            new_window_options: self.new_window_options,
            configuration_error,
            diagnostics: self.diagnostics,
            root: self.root,
            background: self.background,
            close_policy: self.close_policy,
            close_policy_set: self.close_policy_set,
            route_options: self.route_options,
            quit_guards: self.quit_guards,
            routes: Some(Rc::new(routes)),
            initial: None,
            router: self.router,
            web: self.web,
            guards: Vec::new(),
            admissions: Vec::new(),
            control: self.control,
            commands: self.commands,
            #[cfg(feature = "experimental-view")]
            themes: self.themes,
            #[cfg(feature = "updates")]
            distribution: self.distribution,
        }
    }
}

impl<R: Route> App<R> {
    /// Captures the consuming application's build-script update policy and resource layout.
    /// `#[rake::app]` supplies this automatically; manual entry points can call it too.
    #[must_use]
    pub fn distribution_policy(
        self,
        policy: Option<&'static str>,
        resources: &'static str,
    ) -> Self {
        #[cfg(feature = "updates")]
        {
            let mut app = self;
            app.distribution = policy.map(|policy| (policy, resources));
            app
        }
        #[cfg(not(feature = "updates"))]
        {
            let _ = (policy, resources);
            self
        }
    }

    /// Replaces the screen factory without changing destinations, guards, or admissions.
    #[must_use]
    pub fn replace_routes(mut self, routes: impl RouteViews<R>) -> Self {
        self.routes = Some(Rc::new(routes));
        self
    }

    /// Mounts one persistent layout entity per window, outside route history.
    /// Configure this after `routes`. Guard decisions and lifecycle UI remain active.
    #[must_use]
    pub fn layout<L: Render>(
        self,
        mount: impl Fn(crate::router::LayoutContext<R>, &mut Window, &mut gpui::App) -> gpui::Entity<L>
        + 'static,
    ) -> Self {
        self.try_layout(move |context, window, cx| Ok(mount(context, window, cx)))
    }

    /// Mounts a persistent layout that can report service or initialization failures.
    /// Each secondary-window failure rolls back that window's opening operation.
    #[must_use]
    pub fn try_layout<L: Render>(
        mut self,
        mount: impl Fn(
            crate::router::LayoutContext<R>,
            &mut Window,
            &mut gpui::App,
        ) -> Result<gpui::Entity<L>, crate::router::MountError>
        + 'static,
    ) -> Self {
        self.route_options = true;
        self.layout = Some(Rc::new(move |context, window, cx| {
            mount(context, window, cx).map(Into::into)
        }));
        self
    }

    /// Configures secondary native windows using their reserved runtime identity.
    /// The requested navigation activation still determines keyboard focus.
    /// Initial-window options are configured separately with `window_options`.
    #[must_use]
    pub fn new_window_options(
        mut self,
        options: impl Fn(crate::router::WindowId) -> WindowOptions + 'static,
    ) -> Self {
        self.route_options = true;
        self.new_window_options = Some(Rc::new(options));
        self
    }

    /// Registers a concrete shared local service before startup. Duplicates fail explicitly.
    pub fn service<T: 'static>(mut self, service: T) -> Result<Self, ServiceError> {
        self.services = self.services.with(service)?;

        Ok(self)
    }

    /// Installs an owned theme once on the UI thread before the first window mounts.
    ///
    /// Available with `experimental-view`. Calling this before or after `routes`
    /// has the same effect. Multiple theme types coexist; for repeated types the
    /// last registration wins. No `Clone` bound is required. Later theme changes
    /// use [`crate::theme::install`] and are not reset by navigation.
    ///
    /// ```no_run
    /// struct Colors { accent: u32 }
    /// let app = rake::App::new("example.theme").theme(Colors { accent: 0x365849 });
    /// ```
    #[cfg(feature = "experimental-view")]
    #[must_use]
    pub fn theme<T: 'static>(mut self, theme: T) -> Self {
        self.themes
            .push(Box::new(move |cx| crate::theme::install(cx, theme)));
        self
    }

    /// Configures the initial GPUI window; browser hosts apply supported options.
    pub fn window_options(mut self, options: WindowOptions) -> Self {
        self.window_options = options;

        self
    }

    /// Shows the default routing tab/quit controls. Disable for an application-owned
    /// shell; shared guard decisions, errors, lifecycle and quit shortcuts remain active.
    pub fn workspace_chrome(mut self, visible: bool) -> Self {
        self.route_options = true;
        self.workspace_chrome = visible;

        self
    }

    /// Shows the default navigation feedback boundary for routed applications.
    ///
    /// Disable this when an application-owned shell renders its own navigation
    /// feedback. Custom shells can use the public [`crate::router::NavigationBoundary`]
    /// or [`crate::router::Router::navigation`] APIs to choose their presentation.
    pub fn navigation_feedback(mut self, visible: bool) -> Self {
        self.route_options = true;
        self.navigation_feedback = visible;

        self
    }

    /// Sets the native initial destination. Browser deep links take precedence.
    pub fn initial(mut self, destination: R) -> Self {
        self.route_options = true;
        self.initial = Some(destination);

        self
    }

    /// Configures browser path/hash routing.
    pub fn web(mut self, config: WebConfig) -> Self {
        self.route_options = true;
        self.web = config;

        self
    }

    /// Registers one typed command on the application's shared registry.
    /// Duplicate IDs and registration limits fail before the application starts.
    pub fn command<C, F, Fut>(self, handler: F) -> Result<Self, crate::control::CommandError>
    where
        C: crate::control::ApplicationCommand,
        F: Fn(C, crate::control::CommandContext) -> Fut + 'static,
        Fut:
            std::future::Future<Output = Result<C::Output, crate::control::CommandError>> + 'static,
    {
        self.commands.register::<C, F, Fut>(handler)?;

        Ok(self)
    }

    /// Supplies explicit typed application commands shared by GUI and runtime clients.
    pub fn commands(mut self, commands: crate::control::Commands) -> Self {
        self.commands = commands;

        self
    }

    /// Opts into development control for commands, work, shutdown and inspection.
    /// Routed applications additionally expose workspace operations.
    /// Both release and debug builds default to disabled.
    pub fn control(mut self, config: crate::control::ControlConfig) -> Self {
        self.control = config;

        self
    }

    /// Configures operation and history retention bounds.
    pub fn router(mut self, config: RouterConfig) -> Self {
        self.router = config;

        self
    }

    /// Registers a portable access guard before mounting the router.
    pub fn entry_guard<F, Fut>(mut self, guard: F) -> Self
    where
        F: Fn(GuardContext<R>) -> Fut + 'static,
        Fut: std::future::Future<Output = GuardResult> + 'static,
    {
        self.route_options = true;
        self.guards
            .push(Rc::new(move |context| Box::pin(guard(context))));

        self
    }

    /// Registers an asynchronous typed route admission policy before mounting the router.
    ///
    /// Admissions run before entry allocation, host creation, and page mounting. They may
    /// allow the requested destination, deny it with a typed reason, or redirect the same
    /// operation to another destination. Configure this after [`App::routes`].
    #[must_use]
    pub fn entry_admission<F, Fut>(mut self, admission: F) -> Self
    where
        F: Fn(crate::router::AdmissionContext<R>) -> Fut + 'static,
        Fut: std::future::Future<Output = crate::router::Admission<R>> + 'static,
    {
        self.route_options = true;
        self.admissions
            .push(Rc::new(move |context| Box::pin(admission(context))));

        self
    }

    /// Configures the shared request transport.
    pub fn http(mut self, config: HttpConfig) -> Self {
        self.http = config;
        self.http_client = RefCell::new(None);

        self
    }

    /// Configures collection and tracing subscriber ownership.
    pub fn diagnostics(mut self, config: DiagnosticsConfig) -> Self {
        self.diagnostics = config;
        self.http_client = RefCell::new(None);

        self
    }

    /// Registers the initial GPUI view factory, invoked once on the UI event loop.
    pub fn root(
        mut self,
        mount: impl FnOnce(Services, &mut Window, &mut gpui::App) -> AnyView + 'static,
    ) -> Self {
        self.root = Some(Box::new(move |runtime, window, cx| {
            Ok(mount(runtime.services(), window, cx))
        }));

        self
    }

    /// Mounts a standalone view with managed tasks, commands, quit and update actions.
    /// The context is weak: retaining it never keeps a closed runtime alive.
    #[must_use]
    pub fn root_with(
        mut self,
        mount: impl FnOnce(crate::runtime::RuntimeContext, &mut Window, &mut gpui::App) -> AnyView
        + 'static,
    ) -> Self {
        self.root = Some(Box::new(move |runtime, window, cx| {
            Ok(mount(runtime, window, cx))
        }));
        self
    }

    /// Mounts a services-only root whose initialization can fail without panicking.
    #[must_use]
    pub fn try_root(
        mut self,
        mount: impl FnOnce(
            Services,
            &mut Window,
            &mut gpui::App,
        ) -> Result<AnyView, crate::router::MountError>
        + 'static,
    ) -> Self {
        self.root = Some(Box::new(move |runtime, window, cx| {
            mount(runtime.services(), window, cx)
        }));
        self
    }

    /// Mounts a managed root whose initialization can fail without panicking.
    /// Native startup returns `AppError::Mount`; browser hosts show a recovery view.
    #[must_use]
    pub fn try_root_with(
        mut self,
        mount: impl FnOnce(
            crate::runtime::RuntimeContext,
            &mut Window,
            &mut gpui::App,
        ) -> Result<AnyView, crate::router::MountError>
        + 'static,
    ) -> Self {
        self.root = Some(Box::new(mount));
        self
    }

    /// Starts the native application runtime without opening an initial window.
    /// Use the GPUI app context to install a tray or other platform-owned UI.
    /// Browser hosts reject this mode explicitly.
    #[must_use]
    pub fn background(
        mut self,
        setup: impl FnOnce(crate::runtime::RuntimeContext, &mut gpui::App) + 'static,
    ) -> Self {
        self.background = Some(Box::new(setup));
        self
    }

    /// Chooses standalone window close behavior consistently across native platforms.
    #[must_use]
    pub fn close_policy(mut self, policy: ClosePolicy) -> Self {
        self.close_policy = policy;
        self.close_policy_set = true;
        self
    }

    /// Configures managed work and operation bounds for either application shape.
    #[must_use]
    pub fn runtime_config(self, config: RouterConfig) -> Self {
        self.router(config)
    }

    /// Registers an application-wide asynchronous shutdown check before startup.
    /// Guards apply to standalone, background and routed apps, including update restart.
    #[must_use]
    pub fn quit_guard<F, Fut>(mut self, guard: F) -> Self
    where
        F: Fn(crate::quit::QuitGuardContext) -> Fut + 'static,
        Fut: std::future::Future<Output = Result<(), crate::router::GuardReason>> + 'static,
    {
        self.quit_guards
            .push(Rc::new(move |context| Box::pin(guard(context))));
        self
    }

    /// Resolves shared services without initializing diagnostics or a GPU.
    /// Repeated calls and startup reuse the same HTTP transport. Changing HTTP or
    /// diagnostics configuration creates a new transport; older snapshots stay valid.
    pub fn services(&self) -> Result<Services, AppError> {
        if self.id.is_empty() || self.id.chars().any(char::is_whitespace) {
            return Err(AppError::InvalidId);
        }

        let cached = self.http_client.borrow().clone();
        let http = if let Some(http) = cached {
            http
        } else {
            let http =
                HttpClient::with_collector(self.http.clone(), self.diagnostics.shared_collector())?;
            *self.http_client.borrow_mut() = Some(http.clone());
            http
        };
        Ok(Services::new(
            http,
            self.diagnostics.shared_collector(),
            self.services.clone(),
        ))
    }

    /// Installs services and launches the selected GPUI platform.
    ///
    /// Validation, subscriber conflicts and synchronous native UI startup failures
    /// return errors. Browser graphics initialization is asynchronous; startup
    /// failures show a recovery view when a document is available.
    pub fn run(mut self) -> Result<(), AppError> {
        #[cfg(target_family = "wasm")]
        if BROWSER_APP.with(|app| app.borrow().is_some()) {
            return Err(AppError::AlreadyRunning);
        }

        #[cfg(all(feature = "updates", not(target_family = "wasm")))]
        {
            self.services = update_services::install(self.services, self.distribution.take())?;
        }
        if let Some(error) = self.configuration_error {
            return Err(AppError::InvalidConfiguration(error));
        }
        let services = self.services()?;

        if usize::from(self.root.is_some())
            + usize::from(self.routes.is_some())
            + usize::from(self.background.is_some())
            > 1
        {
            return Err(AppError::ConflictingRoots);
        }

        if self.routes.is_some() && self.close_policy_set {
            return Err(AppError::InvalidConfiguration(
                "close_policy applies to standalone windows; routed windows own tab disposal",
            ));
        }
        if self.routes.is_none() && self.route_options {
            return Err(AppError::RoutingRequiresRoutes);
        }
        #[cfg(target_family = "wasm")]
        if self.background.is_some() {
            return Err(AppError::BackgroundUnsupported);
        }

        #[cfg(target_family = "wasm")]
        {
            self.web = crate::router::web::browser::configuration(self.web)?;
        }

        let checkpoint = self
            .routes
            .as_ref()
            .and_then(|_| crate::reload::host::load(self.control));

        let managed = managed::ManagedConfig {
            config: self.router.clone(),
            control: self.control,
            commands: self.commands.clone(),
            application_id: self.id.clone(),
            close_policy: self.close_policy,
            quit_guards: self.quit_guards.clone(),
        };
        let background_config = self.background.as_ref().map(|_| managed.clone());
        let root: Option<WindowRoot> = if let Some(routes) = self.routes.take() {
            #[cfg(not(target_family = "wasm"))]
            let initial = {
                if let Some(initial) = &self.initial {
                    initial.to_uri()?;
                }
                let destination = self
                    .initial
                    .take()
                    .map(Ok)
                    .unwrap_or_else(|| R::from_uri("/"));

                if checkpoint.is_some() {
                    destination.ok()
                } else {
                    Some(destination?)
                }
            };

            #[cfg(target_family = "wasm")]
            let initial = {
                let destination = crate::router::web::browser::location::<R>(&self.web);

                if checkpoint.is_some() {
                    destination.ok()
                } else {
                    Some(destination?)
                }
            };

            let config = self.router;
            let web = self.web;
            let guards = self.guards;
            let admissions = self.admissions;
            let control = self.control;
            let commands = self.commands;
            let application_id = self.id.clone();
            let chrome = self.workspace_chrome;
            let navigation_feedback = self.navigation_feedback;
            let quit_guards = self.quit_guards;
            let layout = self.layout;
            let new_window_options = self.new_window_options;

            Some(Box::new(move |services, window, cx| {
                let root = crate::router::view::RoutingRoot {
                    views: routes,
                    initial,
                    checkpoint,
                    config,
                    web,
                    guards,
                    admissions,
                    control,
                    commands,
                    application_id,
                    chrome,
                    navigation_feedback,
                    quit_guards,
                    layout,
                    new_window_options,
                };

                crate::router::view::Workspace::mount(services, root, window, cx)
            }))
        } else if let Some(mount) = self.root.take() {
            Some(Box::new(move |services, window, cx| {
                managed::mount(mount, services, managed, window, cx)
            }))
        } else if self.background.is_some() {
            None
        } else {
            return Err(AppError::MissingRoot);
        };

        self.diagnostics.install()?;

        #[cfg(target_family = "wasm")]
        console_error_panic_hook::set_once();
        #[cfg(target_family = "wasm")]
        let application = gpui_platform::single_threaded_web();

        #[cfg(not(target_family = "wasm"))]
        let application = gpui_platform::application();

        // Native GPUI image/network hooks use the same observed transport.
        #[cfg(not(target_family = "wasm"))]
        let application = application.with_http_client(Arc::new(GpuiHttp {
            follow: services.http(),
            manual: services.http().without_redirects()?,
        }));

        let startup_errors = startup::StartupErrors::default();
        let errors = startup_errors.clone();
        let launch = move |cx: &mut gpui::App| {
            crate::info!(application = %self.id, "Application starting");

            #[cfg(feature = "experimental-view")]
            install_themes(self.themes, cx);

            #[cfg(all(feature = "updates", not(target_family = "wasm")))]
            if let Ok(updates) = services.get::<rake_updates::UpdateController>() {
                #[cfg(all(feature = "updates-sparkle", target_os = "macos"))]
                if let Err(error) = updates.initialize_sparkle_checks() {
                    tracing::error!(%error, "native update checks could not start");
                }
                let executor = cx.background_executor().clone();
                let task = cx.spawn(async move |_| {
                    updates
                        .run(move |work| {
                            executor.spawn(async move {
                                work.run();
                            })
                        })
                        .await;
                });
                cx.set_global(UpdatesDriver { _task: task });
            }
            let Some(root) = root else {
                if let Some(setup) = self.background {
                    let managed = background_config
                        .expect("background configuration validated before launch");
                    managed::background(setup, services, managed, cx);
                }
                return;
            };
            let mount_errors = errors.clone();
            let opened = cx.open_window(self.window_options, move |window, cx| {
                let view = match root(services, window, cx) {
                    Ok(view) => view,
                    Err(error) => {
                        let error = AppError::Mount(error);
                        mount_errors.record(error);
                        cx.new(|_| gpui::Empty).into()
                    }
                };
                cx.new(|_| RootView(view))
            });
            if let Err(error) = opened {
                errors.record(AppError::Window(error.to_string()));
            }
            if errors.failed() {
                #[cfg(target_os = "macos")]
                rake_updates::stop_application_run_loop()
                    .expect("native GPUI startup runs on the initialized AppKit main thread");
                #[cfg(all(not(target_family = "wasm"), not(target_os = "macos")))]
                cx.quit();
            } else {
                cx.activate(true);
            }
        };

        #[cfg(not(target_family = "wasm"))]
        application.run(launch);
        #[cfg(target_family = "wasm")]
        BROWSER_APP.with(|app| {
            *app.borrow_mut() = Some(application.run_embedded(launch));
        });

        let result = startup_errors.finish();
        #[cfg(target_family = "wasm")]
        if result.is_err() {
            BROWSER_APP.with(|app| {
                app.borrow_mut().take();
            });
        }
        result
    }
}

/// Application configuration and synchronous host initialization failures.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// A typed service could not be registered or resolved during application setup.
    #[error(transparent)]
    Service(#[from] ServiceError),
    /// An application command could not be registered during setup.
    #[error(transparent)]
    Command(#[from] crate::control::CommandError),
    /// Builder ordering would discard configuration belonging to another route type.
    #[error("invalid application configuration: {0}")]
    InvalidConfiguration(&'static str),
    /// The initial root or workspace could not initialize.
    #[error("application root initialization failed: {0}")]
    Mount(#[from] crate::router::MountError),
    /// The host could not create its initial window.
    #[error("initial window creation failed: {0}")]
    Window(String),
    /// Installed update metadata disagrees with the compiled application policy.
    #[cfg(feature = "updates")]
    #[error(transparent)]
    Updates(#[from] rake_updates::Error),
    /// A route-only builder option was supplied to a standalone app.
    #[error("routing options require App::routes")]
    RoutingRequiresRoutes,
    /// Background native apps have no browser host equivalent.
    #[error("background applications require a native host")]
    BackgroundUnsupported,
    /// Legacy error retained for source compatibility; standalone apps now support commands.
    #[deprecated(note = "standalone application commands are supported")]
    /// Registered application commands require a typed router operation owner.
    #[error("application commands require App::routes; App::root supports inspection only")]
    ControlRequiresRouter,
    /// Configure a root-only view or a typed route registry, rather than both.
    #[error("choose only one of App::root, App::root_with, App::routes or App::background")]
    ConflictingRoots,
    /// The initial deep link could not be decoded.
    #[error(transparent)]
    Route(#[from] crate::router::RouteDecodeError),
    /// The browser host could not initialize routing.
    #[error(transparent)]
    Router(#[from] crate::router::OperationError),
    /// The browser host already owns an application instance.
    #[error("the browser host already has a running Rake application")]
    AlreadyRunning,
    /// Application IDs must be nonempty and contain no whitespace.
    #[error("application ID must be nonempty and contain no whitespace")]
    InvalidId,
    /// An application requires a root view factory or typed route registry.
    #[error("register an initial view using App::root or App::routes")]
    MissingRoot,
    /// HTTP configuration could not construct standard services.
    #[error(transparent)]
    Http(#[from] crate::http::Error),
    /// The host's diagnostics setup conflicts with automatic installation.
    #[error(transparent)]
    Diagnostics(#[from] DiagnosticsError),
}

#[cfg(not(target_family = "wasm"))]
struct GpuiHttp {
    follow: HttpClient,
    manual: HttpClient,
}

#[cfg(not(target_family = "wasm"))]
impl gpui::http_client::HttpClient for GpuiHttp {
    fn get(
        &self,
        url: &str,
        follow_redirects: bool,
    ) -> futures::future::BoxFuture<'static, anyhow::Result<gpui::http_client::HttpResponse>> {
        let client = if follow_redirects {
            &self.follow
        } else {
            &self.manual
        };

        let request = client.get(url);

        Box::pin(async move {
            let response = request.send().await?;
            let status = response.status();
            let body = response.bytes().await?;

            Ok(gpui::http_client::HttpResponse {
                status: status.try_into()?,
                body,
            })
        })
    }
}

struct RootView(AnyView);

impl Render for RootView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.0.clone()
    }
}

#[cfg(all(feature = "updates", not(target_family = "wasm")))]
struct UpdatesDriver {
    _task: gpui::Task<()>,
}
#[cfg(all(feature = "updates", not(target_family = "wasm")))]
impl gpui::Global for UpdatesDriver {}

#[cfg(all(test, feature = "view-test-support", not(target_family = "wasm")))]
mod theme_tests {
    //! Covers: deferred app themes, routing composition and owned non-Clone values.
    //! Real: application configuration and theme globals. Doubles: GPUI TestPlatform.
    use super::*;

    #[test]
    fn fallible_root_reports_service_initialization_failure() {
        struct MissingService;
        let mut app = App::new("example.fallible-root").try_root(|services, _, _| {
            services
                .get::<MissingService>()
                .map_err(|error| crate::router::MountError(error.to_string()))?;
            unreachable!("the service is deliberately missing")
        });
        let services = app.services().unwrap();
        let runtime = crate::runtime::Runtime::new(RouterConfig::default(), services);
        let mount = app.root.take().unwrap();
        let mut cx = gpui::TestAppContext::single();
        cx.add_window(|window, cx| {
            let result = mount(runtime.context(), window, cx);
            assert!(matches!(result, Err(error) if error.0.contains("MissingService")));
            gpui::Empty
        });
    }

    #[test]
    fn app_themes_are_deferred_preserved_by_routes_and_installed_in_order() {
        struct Colors(u32);
        struct Metrics(u32);
        let cx = gpui::TestAppContext::single();
        let views = crate::router::__routes(|_: &NoRoute, _, _, _| {
            Err(crate::router::MountError("unused fixture".into()))
        });
        let app = App::new("example.theme-test")
            .theme(Colors(1))
            .routes(views)
            .theme(Metrics(8))
            .theme(Colors(2));

        assert!(crate::theme::try_current::<Colors>().is_none());
        assert!(crate::theme::try_current::<Metrics>().is_none());
        cx.update(|cx| install_themes(app.themes, cx));
        assert_eq!(crate::theme::current::<Colors>().0, 2);
        assert_eq!(crate::theme::current::<Metrics>().0, 8);

        let standalone = App::new("example.root-theme")
            .theme(Colors(3))
            .root(|_, _, cx| cx.new(|_| gpui::Empty).into());
        assert_eq!(
            crate::theme::current::<Colors>().0,
            2,
            "configuration is deferred"
        );
        cx.update(|cx| install_themes(standalone.themes, cx));
        assert_eq!(crate::theme::current::<Colors>().0, 3);
    }
}

#[cfg(test)]
mod configuration_tests {
    //! Covers: same-type route replacement retains the configured destination and guards.
    //! Real: application builder. Doubles: unused page factory. Requires: no UI startup.
    use super::*;

    #[derive(Clone, crate::Route)]
    enum Destination {
        #[route(id = "main", path = "/main")]
        Main,
    }

    #[test]
    fn service_previews_share_the_transport_across_route_registration() {
        let app = App::new("example.transport");
        let first = app.services().unwrap().http();
        let second = app.services().unwrap().http();
        assert!(first.shares_transport_with(&second));
        let pages = crate::router::__routes(|_: &Destination, _, _, _| {
            Err(crate::router::MountError("unused fixture".into()))
        });
        let app = app.routes(pages);
        assert!(first.shares_transport_with(&app.services().unwrap().http()));
        let app = app.http(HttpConfig::new());
        let changed = app.services().unwrap().http();
        assert!(!first.shares_transport_with(&changed));
        let app = app.diagnostics(DiagnosticsConfig::new());
        assert!(!changed.shares_transport_with(&app.services().unwrap().http()));
    }

    #[test]
    fn replacing_routes_preserves_destination_and_guards() {
        let pages = || {
            crate::router::__routes(|_: &Destination, _, _, _| {
                Err(crate::router::MountError("unused fixture".into()))
            })
        };
        let app = App::new("example.replacement")
            .routes(pages())
            .initial(Destination::Main)
            .entry_guard(|_| async {
                GuardResult::Deny(crate::router::GuardReason("denied".into()))
            })
            .entry_admission(|_| async { crate::router::Admission::Allow })
            .replace_routes(pages());

        assert!(matches!(app.initial, Some(Destination::Main)));
        assert_eq!(app.guards.len(), 1);
        assert_eq!(app.admissions.len(), 1);
    }

    #[test]
    fn route_type_transition_rejects_entry_admission_configured_too_early() {
        let pages = || {
            crate::router::__routes(|_: &Destination, _, _, _| {
                Err(crate::router::MountError("unused fixture".into()))
            })
        };
        let app = App::new("example.admission-order")
            .entry_admission(|_| async { crate::router::Admission::Allow })
            .routes(pages());

        assert!(matches!(app.run(), Err(AppError::InvalidConfiguration(_))));
    }

    #[test]
    fn navigation_feedback_defaults_on_and_survives_route_transition() {
        let pages = || {
            crate::router::__routes(|_: &Destination, _, _, _| {
                Err(crate::router::MountError("unused fixture".into()))
            })
        };

        let default = App::new("example.navigation-default").routes(pages());
        assert!(default.navigation_feedback);

        let custom = App::new("example.navigation-custom")
            .navigation_feedback(false)
            .routes(pages());
        assert!(!custom.navigation_feedback);
    }
}
