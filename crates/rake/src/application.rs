//! Application configuration and the native/browser GPUI bootstrap boundary.

#[cfg(not(target_family = "wasm"))]
use std::sync::Arc;

use gpui::{AnyView, AppContext, Context, IntoElement, Render, Window, WindowOptions};

use crate::diagnostics::{Collector, DiagnosticsConfig, DiagnosticsError};
use crate::http::{HttpClient, HttpConfig};
use crate::router::runtime::Guard;

use crate::router::{
    GuardContext, GuardResult, NoRoute, Route, RouteViews, RouterConfig, WebConfig,
};

use std::rc::Rc;

/// Standard application services, cheaply cloned without retaining a view entity.
#[derive(Clone)]
pub struct Services {
    http: HttpClient,
    collector: Collector,
}

impl Services {
    /// Returns the shared configured HTTP transport handle.
    pub fn http(&self) -> HttpClient {
        self.http.clone()
    }

    /// Returns the bounded diagnostic collector.
    pub fn diagnostics(&self) -> Collector {
        self.collector.clone()
    }
}

type Root = Box<dyn FnOnce(Services, &mut Window, &mut gpui::App) -> AnyView>;

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
}

impl App<NoRoute> {
    /// Creates an application. Its nonempty, whitespace-free ID is validated at startup.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
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
        }
    }
}

impl<R: Route> App<R> {
    /// Registers the application's typed screen factory. Defaults to the `/` route.
    pub fn routes<Next: Route>(self, routes: impl RouteViews<Next>) -> App<Next> {
        App {
            id: self.id,
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
        }
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

    /// Supplies explicit typed application commands shared by GUI and runtime clients.
    pub fn commands(mut self, commands: crate::control::Commands) -> Self {
        self.commands = commands;

        self
    }

    /// Opts into development control. Both release and debug builds default to disabled.
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

        Ok(Services {
            http: HttpClient::with_collector(
                self.http.clone(),
                self.diagnostics.collector.clone(),
            )?,
            collector: self.diagnostics.collector.clone(),
        })
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

        let services = self.services()?;

        if self.root.is_some() && self.routes.is_some() {
            return Err(AppError::ConflictingRoots);
        }

        if self.routes.is_none()
            && (!self.commands.descriptors().is_empty()
                || !matches!(self.control, crate::control::ControlConfig::Disabled))
        {
            return Err(AppError::ControlRequiresRouter);
        }

        let checkpoint = crate::reload::host::load(self.control);

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
                let destination = crate::router::web::browser::location::<R>(self.web.routing);

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
            self.root.take().ok_or(AppError::MissingRoot)?
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

            match cx.open_window(WindowOptions::default(), move |window, cx| {
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
    /// The initial runtime control adapter requires a typed router owner.
    #[error("runtime control and application commands require App::routes")]
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
