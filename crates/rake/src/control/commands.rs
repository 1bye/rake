//! Explicit typed application command registration, independent of UI and transport.

use crate::http::CancellationToken;
use crate::tasks::AppHandle;
use futures::future::LocalBoxFuture;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::future::Future;
use std::rc::Rc;

/// Capability required before a registered handler is invoked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionCapability {
    /// Services and owned domain data suffice without any mounted UI.
    Headless,
    /// This handler requires a live application window.
    RequiresWindow,
}

/// Argument/result JSON Schemas and execution requirement.
/// Derived commands generate schemas from their data types; manual implementations
/// remain available for custom codecs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommandDescriptor {
    /// Stable external command key, independent of Rust type name.
    pub id: String,
    /// JSON schema for arguments; typed Serde decoding is authoritative.
    pub arguments: Value,
    /// JSON schema for the committed result.
    pub result: Value,
    /// Headless or window-dependent execution.
    pub capability: ExecutionCapability,
}

impl CommandDescriptor {
    /// Generates Draft 2020-12 schemas for accepted arguments and serialized results.
    /// The two contracts deliberately differ for `skip_serializing`, defaults, etc.
    pub fn for_type<C>(id: &str, capability: ExecutionCapability) -> Self
    where
        C: ApplicationCommand + schemars::JsonSchema,
        C::Output: schemars::JsonSchema,
    {
        let arguments = schemars::generate::SchemaSettings::draft2020_12()
            .for_deserialize()
            .into_generator()
            .into_root_schema_for::<C>();

        let result = schemars::generate::SchemaSettings::draft2020_12()
            .for_serialize()
            .into_generator()
            .into_root_schema_for::<C::Output>();

        Self {
            id: id.into(),
            arguments: arguments.to_value(),
            result: result.to_value(),
            capability,
        }
    }
}

/// Typed arguments for an explicitly registered application operation.
/// Derive with `#[rake::data(schema)]` and `#[derive(rake::Command)]`, or supply
/// manual metadata for custom codecs. Schema annotations describe contracts;
/// Serde decoding and handler validation remain authoritative.
///
/// ```
/// use rake::control::{ApplicationCommand, ExecutionCapability};
/// #[rake::data(schema)]
/// #[derive(rake::Command)]
/// #[command(id = "document.title", output = String, capability = Headless)]
/// #[serde(deny_unknown_fields)]
/// struct Title { document: String }
/// assert_eq!(Title::id(), "document.title");
/// assert_eq!(Title::descriptor().result["type"], "string");
/// assert_eq!(Title::descriptor().capability, ExecutionCapability::Headless);
/// ```
///
/// Duplicate metadata is rejected by the derive:
///
/// ```compile_fail
/// #[rake::data(schema)]
/// #[derive(rake::Command)]
/// #[command(id = "one", id = "two")]
/// struct Ambiguous;
/// ```
///
/// Commands have a closed schema and stable identity, rather than generic variants:
///
/// ```compile_fail
/// #[rake::data(schema)]
/// #[derive(rake::Command)]
/// #[command(id = "generic")]
/// struct Generic<T> { argument: T }
/// ```
///
/// The schema flag is required unless JSON Schema is implemented separately:
///
/// ```compile_fail
/// #[rake::data]
/// #[derive(rake::Command)]
/// #[command(id = "missing.schema")]
/// struct MissingSchema { value: String }
/// ```
pub trait ApplicationCommand: Serialize + DeserializeOwned + 'static {
    /// Serializable committed result.
    type Output: Serialize + DeserializeOwned + 'static;

    /// Stable command key. Derives return it without rebuilding JSON Schemas.
    fn id() -> String {
        Self::descriptor().id
    }

    /// Stable command metadata and schemas.
    fn descriptor() -> CommandDescriptor;
}

/// Owned handler context. Futures are local and no borrowed UI context crosses await.
#[derive(Clone)]
pub struct CommandContext {
    /// The same application-owned typed service values supplied to routed views.
    pub services: crate::services::ServiceRegistry,
    /// Application task scope, independent of any currently selected tab.
    pub app: AppHandle,
    /// Cancellation signal for this operation; cancellation cannot undo server effects.
    pub cancellation: CancellationToken,
}

impl CommandContext {
    /// Returns the application-owned HTTP handle when standard services were supplied.
    ///
    /// # Errors
    /// Returns [`crate::services::ServiceError::Missing`] for a raw registry without
    /// standard application services.
    pub fn http(&self) -> Result<crate::http::HttpClient, crate::services::ServiceError> {
        self.services.http()
    }

    /// Returns the application-owned diagnostic collector when standard services were supplied.
    ///
    /// # Errors
    /// Returns [`crate::services::ServiceError::Missing`] for a raw registry without
    /// standard application services.
    pub fn diagnostics(
        &self,
    ) -> Result<crate::diagnostics::Collector, crate::services::ServiceError> {
        self.services.diagnostics()
    }

    /// Reconstructs the standard service view for this command's application composition.
    ///
    /// # Errors
    /// Returns [`crate::services::ServiceError::Missing`] for a raw registry without
    /// standard application services.
    pub fn app_services(&self) -> Result<crate::services::Services, crate::services::ServiceError> {
        self.services.standard_services()
    }
}

use rake_protocol::control::CommandError;

type Handler = Rc<
    dyn Fn(
        Value,
        CommandContext,
    ) -> Result<LocalBoxFuture<'static, Result<Value, CommandError>>, CommandError>,
>;

/// Presentation shared by command menus and keyboard bindings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandPresentation {
    /// Human-readable menu label.
    pub label: String,
    /// GPUI keystroke (for example `cmd-s`), bound globally by the menu host.
    pub shortcut: Option<String>,
}

/// Current availability and toggle state, shared by every command entry point.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandState {
    /// Whether invocation is permitted.
    pub enabled: bool,
    /// Whether a menu checkmark is displayed.
    pub checked: bool,
    /// Optional current label, overriding the presentation label.
    pub label: Option<String>,
}

impl Default for CommandState {
    fn default() -> Self {
        Self {
            enabled: true,
            checked: false,
            label: None,
        }
    }
}

struct Registered {
    descriptor: CommandDescriptor,
    handler: Handler,
    presentation: CommandPresentation,
    state: CommandState,
}

/// Explicit registry shared by GUI, Rust client and external runtime messages.
/// Registration is bounded at 256 commands; replacing a handler is not implicit.
#[derive(Clone, Default)]
pub struct Commands(
    Rc<RefCell<BTreeMap<String, Registered>>>,
    Rc<event_listener::Event>,
);

impl Commands {
    /// An empty command registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers typed local-future arguments/results without a `Send` requirement.
    /// Deserialization happens before invoking the user handler.
    pub fn register<C, F, Fut>(&self, handler: F) -> Result<(), CommandError>
    where
        C: ApplicationCommand,
        F: Fn(C, CommandContext) -> Fut + 'static,
        Fut: Future<Output = Result<C::Output, CommandError>> + 'static,
    {
        let descriptor = C::descriptor();
        let mut registry = self.0.borrow_mut();

        if descriptor.id.is_empty()
            || descriptor.id.len() > 128
            || descriptor.id.chars().any(char::is_whitespace)
            || registry.len() >= 256
            || registry.contains_key(&descriptor.id)
        {
            return Err(CommandError::Registration(descriptor.id));
        }

        let handler = Rc::new(handler);

        let handler = Rc::new(move |arguments, context| {
            let arguments = serde_json::from_value::<C>(arguments)
                .map_err(|error| CommandError::Arguments(error.to_string()))?;

            let handler = handler.clone();

            let future: LocalBoxFuture<'static, Result<Value, CommandError>> =
                Box::pin(async move {
                    serde_json::to_value(handler(arguments, context).await?)
                        .map_err(|error| CommandError::Result(error.to_string()))
                });

            Ok(future)
        });

        registry.insert(
            descriptor.id.clone(),
            Registered {
                presentation: CommandPresentation {
                    label: descriptor.id.clone(),
                    shortcut: None,
                },
                state: CommandState::default(),
                descriptor,
                handler,
            },
        );

        Ok(())
    }

    /// Sets command presentation and wakes mounted menu hosts, including shortcut updates.
    /// # Errors
    /// Returns `Unknown` for an unregistered stable ID, or `Arguments` for an empty
    /// shortcut (and, with GPUI enabled, invalid keystroke syntax).
    pub fn set_presentation(
        &self,
        id: &str,
        presentation: CommandPresentation,
    ) -> Result<(), CommandError> {
        // Check identity before validating metadata so unknown IDs remain unambiguous.
        self.presentation(id)?;

        if let Some(shortcut) = &presentation.shortcut {
            if shortcut.trim().is_empty() {
                return Err(CommandError::Arguments(
                    "command shortcut cannot be empty".into(),
                ));
            }

            #[cfg(feature = "gpui")]
            for stroke in shortcut.split_whitespace() {
                let os = if cfg!(target_os = "macos") {
                    gpui::OperatingSystem::Mac
                } else if cfg!(target_os = "windows") {
                    gpui::OperatingSystem::Windows
                } else {
                    gpui::OperatingSystem::Linux
                };

                gpui::Keystroke::parse_with_os(stroke, os).map_err(|error| {
                    CommandError::Arguments(format!("invalid shortcut for {id}: {error}"))
                })?;
            }
        }

        self.0
            .borrow_mut()
            .get_mut(id)
            .ok_or_else(|| CommandError::Unknown(id.into()))?
            .presentation = presentation;
        self.1.notify(usize::MAX);

        Ok(())
    }

    /// Updates availability, checkmark and label, waking mounted menu hosts.
    /// Disabled commands are rejected through GUI, CLI and agent entry points alike.
    /// # Errors
    /// Returns `Unknown` for an unregistered stable ID.
    pub fn set_state(&self, id: &str, state: CommandState) -> Result<(), CommandError> {
        self.0
            .borrow_mut()
            .get_mut(id)
            .ok_or_else(|| CommandError::Unknown(id.into()))?
            .state = state;
        self.1.notify(usize::MAX);

        Ok(())
    }

    /// Reads the current presentation and state for a registered stable ID.
    /// # Errors
    /// Returns `Unknown` for an unregistered stable ID.
    pub fn presentation(
        &self,
        id: &str,
    ) -> Result<(CommandPresentation, CommandState), CommandError> {
        self.0
            .borrow()
            .get(id)
            .map(|command| (command.presentation.clone(), command.state.clone()))
            .ok_or_else(|| CommandError::Unknown(id.into()))
    }

    #[cfg(feature = "gpui")]
    pub(crate) fn listen(&self) -> event_listener::EventListener {
        self.1.listen()
    }

    /// Copies the stable application command schemas.
    pub fn descriptors(&self) -> Vec<CommandDescriptor> {
        self.0
            .borrow()
            .values()
            .map(|command| command.descriptor.clone())
            .collect()
    }

    pub(super) fn prepare(
        &self,
        id: &str,
        arguments: Value,
        window: bool,
        context: CommandContext,
    ) -> Result<LocalBoxFuture<'static, Result<Value, CommandError>>, CommandError> {
        let (capability, enabled, handler) = self
            .0
            .borrow()
            .get(id)
            .map(|command| {
                (
                    command.descriptor.capability,
                    command.state.enabled,
                    command.handler.clone(),
                )
            })
            .ok_or_else(|| CommandError::Unknown(id.into()))?;

        if !enabled {
            return Err(CommandError::Failed(format!("command is disabled: {id}")));
        }

        if capability == ExecutionCapability::RequiresWindow && !window {
            return Err(CommandError::RequiresWindow);
        }

        handler(arguments, context)
    }
}
