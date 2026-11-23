//! Typed command and control handles with the application's GPUI services.

use std::future::Future;

use super::{
    ApplicationCommand, CommandDescriptor, CommandError, CommandPresentation, CommandState,
    ControlError, Reply, Request, RuntimeInfo, RuntimeWatch,
};

use crate::router::Route;
use crate::services::{ServiceError, ServiceRegistry, Services};

/// Owned application command context, safe to retain across awaits.
#[derive(Clone)]
pub struct CommandContext {
    /// The same typed service values supplied to routed views.
    pub services: ServiceRegistry,
    /// Application task scope, independent of any selected tab.
    pub app: crate::tasks::AppHandle,
    /// Cancellation signal for this operation.
    pub cancellation: crate::http::CancellationToken,
}

impl CommandContext {
    fn from_core(context: rake_core::control::CommandContext) -> Self {
        Self {
            services: ServiceRegistry::from_core(context.services),
            app: context.app,
            cancellation: context.cancellation,
        }
    }

    /// Returns the standard application HTTP handle, when present.
    pub fn http(&self) -> Result<crate::http::HttpClient, ServiceError> {
        self.services.http()
    }

    /// Returns the standard application diagnostic collector, when present.
    pub fn diagnostics(&self) -> Result<crate::diagnostics::Collector, ServiceError> {
        self.services.diagnostics()
    }

    /// Reconstructs the standard service view for this command.
    pub fn app_services(&self) -> Result<Services, ServiceError> {
        self.services.standard_services()
    }
}

/// Registry shared by GUI dispatch and the runtime control protocol.
#[derive(Clone)]
pub struct Commands(rake_core::control::Commands);

impl Default for Commands {
    fn default() -> Self {
        Self::new()
    }
}

impl Commands {
    /// An empty command registry.
    pub fn new() -> Self {
        Self::from_core(rake_core::control::Commands::new())
    }

    pub(crate) fn from_core(core: rake_core::control::Commands) -> Self {
        Self(core.with_shortcut_validation(validate_shortcut))
    }

    pub(crate) fn into_core(self) -> rake_core::control::Commands {
        self.0
    }

    /// Registers a typed command whose future runs on the local executor.
    pub fn register<C, F, Fut>(&self, handler: F) -> Result<(), CommandError>
    where
        C: ApplicationCommand,
        F: Fn(C, CommandContext) -> Fut + 'static,
        Fut: Future<Output = Result<C::Output, CommandError>> + 'static,
    {
        self.0.register::<C, _, _>(move |args, context| {
            handler(args, CommandContext::from_core(context))
        })
    }

    /// Sets command presentation and wakes mounted menu hosts.
    pub fn set_presentation(
        &self,
        id: &str,
        presentation: CommandPresentation,
    ) -> Result<(), CommandError> {
        self.0.set_presentation(id, presentation)
    }

    /// Updates availability, checkmark and label.
    pub fn set_state(&self, id: &str, state: CommandState) -> Result<(), CommandError> {
        self.0.set_state(id, state)
    }

    /// Returns a command's presentation and state.
    pub fn presentation(
        &self,
        id: &str,
    ) -> Result<(CommandPresentation, CommandState), CommandError> {
        self.0.presentation(id)
    }

    /// Lists the registered command descriptors.
    pub fn descriptors(&self) -> Vec<CommandDescriptor> {
        self.0.descriptors()
    }

    pub(crate) fn listen(&self) -> event_listener::EventListener {
        self.0.listen()
    }
}

fn validate_shortcut(id: &str, shortcut: &str) -> Result<(), CommandError> {
    let os = if cfg!(target_os = "macos") {
        gpui::OperatingSystem::Mac
    } else if cfg!(target_os = "windows") {
        gpui::OperatingSystem::Windows
    } else {
        gpui::OperatingSystem::Linux
    };

    for stroke in shortcut.split_whitespace() {
        gpui::Keystroke::parse_with_os(stroke, os).map_err(|error| {
            CommandError::Arguments(format!("invalid shortcut for {id}: {error}"))
        })?;
    }

    Ok(())
}

/// Local client of the authoritative runtime and its GPUI services.
#[derive(Clone)]
pub struct RuntimeClient<R: Route>(rake_core::control::RuntimeClient<R>);

impl<R: Route> RuntimeClient<R> {
    pub(crate) fn from_core(core: rake_core::control::RuntimeClient<R>) -> Self {
        Self(core)
    }

    /// Returns runtime capabilities and retention limits.
    pub fn info(&self) -> RuntimeInfo {
        self.0.info()
    }

    /// Validates and dispatches a runtime request.
    pub fn request(&self, request: Request) -> Result<Reply, ControlError> {
        self.0.request(request)
    }

    /// Dispatches asynchronous storage inspection and ordinary requests.
    #[cfg(feature = "devtools")]
    pub async fn request_async(&self, request: Request) -> Result<Reply, ControlError> {
        self.0.request_async(request).await
    }

    /// Captures a snapshot and starts observing the runtime from its revision.
    pub fn watch(&self) -> RuntimeWatch<R> {
        self.0.watch()
    }

    /// Captures the running application's shared service composition.
    pub fn services(&self) -> ServiceRegistry {
        ServiceRegistry::from_core(self.0.services())
    }

    /// Returns the command registry shared with the application.
    pub fn commands(&self) -> Commands {
        Commands::from_core(self.0.commands())
    }

    /// Dispatches typed command arguments with an explicit retry key.
    pub fn dispatch<C: ApplicationCommand>(
        &self,
        command: C,
        key: String,
    ) -> Result<Reply, ControlError> {
        self.0.dispatch(command, key)
    }

    /// Executes a local typed user action through the shared scheduler.
    pub fn execute<C: ApplicationCommand>(&self, command: C) -> Result<Reply, ControlError> {
        self.0.execute(command)
    }

    #[cfg(any(unix, windows, target_family = "wasm"))]
    pub(crate) fn into_host(self) -> super::HostClient {
        self.0.into_host()
    }

    pub(crate) fn set_commands(&self, commands: Commands) {
        self.0.set_commands(commands.into_core());
    }

    #[cfg(all(test, feature = "view-test-support", not(target_family = "wasm")))]
    pub(crate) fn set_services(&self, services: ServiceRegistry) {
        self.0.set_services(services.into_core());
    }

    pub(crate) fn dispatch_local_command(
        &self,
        id: String,
        arguments: serde_json::Value,
    ) -> Result<Reply, ControlError> {
        self.0.dispatch_local_command(id, arguments)
    }

    #[cfg(not(target_family = "wasm"))]
    pub(crate) async fn responses_idle(&self) {
        self.0.responses_idle().await;
    }

    #[cfg(feature = "devtools")]
    pub(crate) fn inspection(&self) -> crate::inspection::Store {
        self.0
            .inspection_host(crate::inspection::Store::default)
            .clone()
    }
}

#[cfg(any(unix, windows))]
/// Native local control transport.
pub mod local {
    use super::{Route, RuntimeClient};
    use std::{io, path::Path};

    pub use rake_core::control::local::*;

    /// The server and its bound private control socket.
    pub struct LocalServer(rake_core::control::local::LocalServer);

    impl LocalServer {
        /// Binds this client's discovery record and local socket.
        pub fn bind<R: Route>(
            client: &RuntimeClient<R>,
            application_id: &str,
            path: &Path,
        ) -> io::Result<Self> {
            rake_core::control::local::LocalServer::bind(&client.0, application_id, path).map(Self)
        }

        /// Serves requests until the future or runtime is dropped.
        pub async fn run<R: Route>(&self, client: RuntimeClient<R>) -> io::Result<()> {
            self.0.run(client.0).await
        }

        pub(crate) fn bind_host(
            client: &crate::control::HostClient,
            application_id: &str,
            path: &Path,
        ) -> io::Result<Self> {
            rake_core::control::local::LocalServer::bind_host(client, application_id, path)
                .map(Self)
        }

        pub(crate) async fn run_host(&self, client: crate::control::HostClient) -> io::Result<()> {
            self.0.run_host(client).await
        }
    }
}

#[cfg(target_family = "wasm")]
/// Owns a browser's outbound development connection.
pub struct WebBridge(rake_core::control::WebBridge);

#[cfg(target_family = "wasm")]
impl WebBridge {
    /// Uses a dedicated bounded HTTP client for browser control polling.
    pub fn new(config: super::WebBridgeConfig) -> Result<Self, crate::http::Error> {
        rake_core::control::WebBridge::new(config).map(Self)
    }

    /// Runs the connection until this future is dropped.
    pub async fn run<R: Route>(&self, client: RuntimeClient<R>, application_id: &str) {
        self.0.run(client.0, application_id).await;
    }

    pub(crate) async fn run_host(&self, client: super::HostClient, application_id: &str) {
        self.0.run_host(client, application_id).await;
    }
}
