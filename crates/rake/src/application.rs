//! Application configuration and the native/browser GPUI bootstrap boundary.

#[cfg(not(target_family = "wasm"))]
use std::sync::Arc;

use gpui::{AnyView, AppContext, Context, IntoElement, Render, Window, WindowOptions};

use crate::diagnostics::{DiagnosticsConfig, DiagnosticsError};
use crate::http::{HttpClient, HttpConfig};
use crate::router::runtime::Guard;
use crate::services::{ServiceError, ServiceRegistry, Services};

use crate::router::{
    GuardContext, GuardResult, NoRoute, Route, RouteViews, RouterConfig, WebConfig,
};

use std::rc::Rc;

type Root = Box<dyn FnOnce(Services, &mut Window, &mut gpui::App) -> AnyView>;

#[cfg(feature = "experimental-view")]
type ThemeInstaller = Box<dyn FnOnce(&mut gpui::App)>;

#[cfg(feature = "experimental-view")]
fn install_themes(themes: Vec<ThemeInstaller>, cx: &mut gpui::App) {
    for install in themes {
        install(cx);
    }
}

#[cfg(feature = "devtools")]
mod standalone;

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
    http: HttpConfig,
    diagnostics: DiagnosticsConfig,
    root: Option<Root>,
    routes: Option<Rc<dyn RouteViews<R>>>,
    initial: Option<R>,
    router: RouterConfig,
    web: WebConfig,
    guards: Vec<Guard<R>>,
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
            http: HttpConfig::new(),
            diagnostics: DiagnosticsConfig::new(),
            root: None,
            routes: None,
            initial: None,
            router: RouterConfig::default(),
            web: WebConfig::new(),
            guards: Vec::new(),
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
    /// Configure `initial` and `entry_guard` after this type transition. To change
    /// factories for the same destination type, use [`App::replace_routes`], which
    /// preserves the initial destination and guards.
    ///
    /// ```compile_fail
    /// #[derive(Clone, rake::Route)]
    /// enum Destination { #[route(id = "home", path = "/")] Home }
    /// let pages = || rake::router::__routes(|_: &Destination, _, _, _| {
    ///     Err(rake::router::MountError("fixture".into()))
    /// });
    /// let app = rake::App::new("example.routes").routes(pages());
    /// let app = app.routes(pages()); // use replace_routes instead
    /// ```
    pub fn routes<Next: Route>(self, routes: impl RouteViews<Next>) -> App<Next> {
        App {
            id: self.id,
            services: self.services,
            window_options: self.window_options,
            workspace_chrome: self.workspace_chrome,
            http: self.http,
            diagnostics: self.diagnostics,
            root: self.root,
            routes: Some(Rc::new(routes)),
            initial: None,
            router: self.router,
            web: self.web,
            guards: Vec::new(),
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

    /// Replaces the screen factory without changing destinations or access guards.
    #[must_use]
    pub fn replace_routes(mut self, routes: impl RouteViews<R>) -> Self {
        self.routes = Some(Rc::new(routes));
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
        self.workspace_chrome = visible;

        self
    }

    /// Sets the native initial destination. Browser deep links take precedence.
    pub fn initial(mut self, destination: R) -> Self {
        self.initial = Some(destination);

        self
    }

    /// Configures browser path/hash routing.
    pub fn web(mut self, config: WebConfig) -> Self {
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

    /// Opts into development control. Standalone roots expose inspection/diagnostics;
    /// routed applications also expose workspace operations and registered commands.
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
        self.guards
            .push(Rc::new(move |context| Box::pin(guard(context))));

        self
    }

    /// Configures the shared request transport.
    pub fn http(mut self, config: HttpConfig) -> Self {
        self.http = config;

        self
    }

    /// Configures collection and tracing subscriber ownership.
    pub fn diagnostics(mut self, config: DiagnosticsConfig) -> Self {
        self.diagnostics = config;

        self
    }

    /// Registers the initial GPUI view factory, invoked once on the UI event loop.
    pub fn root(
        mut self,
        mount: impl FnOnce(Services, &mut Window, &mut gpui::App) -> AnyView + 'static,
    ) -> Self {
        self.root = Some(Box::new(mount));

        self
    }

    /// Validates service configuration without initializing diagnostics or a GPU.
    pub fn services(&self) -> Result<Services, AppError> {
        if self.id.is_empty() || self.id.chars().any(char::is_whitespace) {
            return Err(AppError::InvalidId);
        }

        Ok(Services::new(
            HttpClient::with_collector(self.http.clone(), self.diagnostics.shared_collector())?,
            self.diagnostics.shared_collector(),
            self.services.clone(),
        ))
    }

    /// Installs services and launches the selected GPUI platform.
    ///
    /// Startup validation and subscriber conflicts return errors. Window creation
    /// failures are emitted as runtime errors and request application shutdown;
    /// browser graphics initialization occurs asynchronously after this returns.
    pub fn run(mut self) -> Result<(), AppError> {
        #[cfg(target_family = "wasm")]
        if BROWSER_APP.with(|app| app.borrow().is_some()) {
            return Err(AppError::AlreadyRunning);
        }

        #[cfg(all(feature = "updates", not(target_family = "wasm")))]
        if self.services.get::<rake_updates::UpdateService>().is_err()
            && let Some((policy, name)) = self.distribution.take()
        {
            match crate::Resources::discover(name) {
                Ok(resources) => {
                    let updates = rake_updates::UpdateService::new(policy, &resources)?;
                    self.services = self.services.with(updates).map_err(|error| {
                        AppError::Updates(rake_updates::Error::Policy(error.to_string()))
                    })?;
                }
                Err(crate::resources::Error::Unbundled) => {}
                Err(error) => {
                    return Err(AppError::Updates(rake_updates::Error::Policy(
                        error.to_string(),
                    )));
                }
            }
        }
        #[cfg(all(feature = "updates", not(target_family = "wasm")))]
        if self
            .services
            .get::<rake_updates::UpdateController>()
            .is_err()
            && let Ok(service) = self.services.get::<rake_updates::UpdateService>()
        {
            self.services = self
                .services
                .with(rake_updates::UpdateController::new((*service).clone()))
                .map_err(|error| {
                    AppError::Updates(rake_updates::Error::Policy(error.to_string()))
                })?;
        }
        let services = self.services()?;

        if self.root.is_some() && self.routes.is_some() {
            return Err(AppError::ConflictingRoots);
        }

        if self.routes.is_none() && !self.commands.descriptors().is_empty() {
            return Err(AppError::ControlRequiresRouter);
        }

        #[cfg(target_family = "wasm")]
        {
            self.web = crate::router::web::browser::configuration(self.web)?;
        }

        let checkpoint = self
            .routes
            .as_ref()
            .and_then(|_| crate::reload::host::load(self.control));

        let root: Root = if let Some(routes) = self.routes.take() {
            #[cfg(not(target_family = "wasm"))]
            let initial = {
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
            let control = self.control;
            let commands = self.commands;
            let application_id = self.id.clone();
            let chrome = self.workspace_chrome;

            Box::new(move |services, window, cx| {
                let root = crate::router::view::RoutingRoot {
                    views: routes,
                    initial,
                    checkpoint,
                    config,
                    web,
                    guards,
                    control,
                    commands,
                    application_id,
                    chrome,
                };

                match crate::router::view::Workspace::mount(services, root, window, cx) {
                    Ok(view) => view,
                    Err(error) => {
                        crate::error!(%error, "Router host mount failed");

                        cx.new(|_| gpui::Empty).into()
                    }
                }
            })
        } else {
            let mount = self.root.take().ok_or(AppError::MissingRoot)?;
            #[cfg(feature = "devtools")]
            let control = self.control;
            #[cfg(feature = "devtools")]
            let application_id = self.id.clone();
            Box::new(move |services, window, cx| {
                #[cfg(feature = "devtools")]
                {
                    let content = mount(services.clone(), window, cx);
                    standalone::mount(content, &services, control, application_id, cx)
                }
                #[cfg(not(feature = "devtools"))]
                {
                    mount(services, window, cx)
                }
            })
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
            match cx.open_window(self.window_options, move |window, cx| {
                let view = root(services, window, cx);

                cx.new(|_| RootView(view))
            }) {
                Ok(_) => cx.activate(true),
                Err(error) => {
                    crate::error!(%error, "Initial window creation failed");
                    cx.quit();
                }
            }
        };

        #[cfg(not(target_family = "wasm"))]
        application.run(launch);
        #[cfg(target_family = "wasm")]
        BROWSER_APP.with(|app| {
            *app.borrow_mut() = Some(application.run_embedded(launch));
        });

        Ok(())
    }
}

/// Application bootstrap errors before the host event loop starts.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// Installed update metadata disagrees with the compiled application policy.
    #[cfg(feature = "updates")]
    #[error(transparent)]
    Updates(#[from] rake_updates::Error),
    /// Registered application commands require a typed router operation owner.
    #[error("application commands require App::routes; App::root supports inspection only")]
    ControlRequiresRouter,
    /// Configure a root-only view or a typed route registry, rather than both.
    #[error("App::root and App::routes cannot both own the root")]
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
            .replace_routes(pages());

        assert!(matches!(app.initial, Some(Destination::Main)));
        assert_eq!(app.guards.len(), 1);
    }
}
