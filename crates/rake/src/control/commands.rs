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

/// Command registration, typed argument, capability or handler failure.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum CommandError {
    /// The stable command key was never registered.
    #[error("unknown application command: {0}")]
    Unknown(String),
    /// Stable key invalid or duplicated.
    #[error("invalid or duplicate application command: {0}")]
    Registration(String),
    /// Typed argument decoding failed.
    #[error("invalid command arguments: {0}")]
    Arguments(String),
    /// A UI-dependent handler cannot execute headlessly.
    #[error("this command requires a window")]
    RequiresWindow,
    /// The handler deliberately reported an application failure.
    #[error("application command failed: {0}")]
    Failed(String),
    /// Typed result could not be serialized.
    #[error("command result serialization failed: {0}")]
    Result(String),
}

type Handler = Rc<
    dyn Fn(
        Value,
        CommandContext,
    ) -> Result<LocalBoxFuture<'static, Result<Value, CommandError>>, CommandError>,
>;

struct Registered {
    descriptor: CommandDescriptor,
    handler: Handler,
}

/// Explicit registry shared by GUI, Rust client and external runtime messages.
/// Registration is bounded at 256 commands; replacing a handler is not implicit.
#[derive(Clone, Default)]
pub struct Commands(Rc<RefCell<BTreeMap<String, Registered>>>);

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
                descriptor,
                handler,
            },
        );

        Ok(())
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
        let (capability, handler) = self
            .0
            .borrow()
            .get(id)
            .map(|command| (command.descriptor.capability, command.handler.clone()))
            .ok_or_else(|| CommandError::Unknown(id.into()))?;

        if capability == ExecutionCapability::RequiresWindow && !window {
            return Err(CommandError::RequiresWindow);
        }

        handler(arguments, context)
    }
}
