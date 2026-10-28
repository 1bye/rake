//! Versioned runtime commands, queries and revisioned state events shared by all clients.
//!
//! Dispatch stays on the host event loop. Transport messages contain owned data and
//! never grant filesystem, process or arbitrary Rust evaluation capabilities.
//! Operation submission is eager; disconnecting only stops observation.
//!
//! ```
//! use rake::control::{Message, Query, Reply, Request, PROTOCOL_VERSION};
//! use rake::router::{NoRoute, Router, RouterConfig};
//! let router = Router::<NoRoute>::new(RouterConfig::default());
//! let reply = router.client().request(Request {
//!     protocol: PROTOCOL_VERSION, runtime_id: router.runtime_id(), key: None,
//!     message: Message::Query(Query::Info {}),
//! }).unwrap();
//! assert!(matches!(reply, Reply::Info(_)));
//! ```

mod commands;
mod protocol;

pub use commands::*;

#[cfg(unix)]
pub mod local;
#[cfg(target_family = "wasm")]
mod web;

pub use protocol::*;
#[cfg(target_family = "wasm")]
pub use web::{WebBridge, WebBridgeConfig};

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use serde_json::json;

use crate::router::{OperationId, Route, Router};

struct SavedRequest {
    command: Command,
    reply: Result<Reply, ControlError>,
}

pub(crate) struct ClientState {
    pub(crate) inspection: crate::inspection::Store,
    commands: RefCell<Commands>,
    services: RefCell<crate::services::ServiceRegistry>,
    responses: Cell<usize>,
    response_event: event_listener::Event,
    capacity: usize,
    requests: RefCell<BTreeMap<String, SavedRequest>>,
}

impl ClientState {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            inspection: crate::inspection::Store::default(),
            commands: RefCell::new(Commands::new()),
            services: RefCell::new(crate::services::ServiceRegistry::new()),
            responses: Cell::new(0),
            response_event: event_listener::Event::new(),
            capacity,
            requests: RefCell::new(BTreeMap::new()),
        }
    }
}

// A native host must let accepted transport responses reach the socket before exit.
#[cfg(any(unix, target_family = "wasm"))]
pub(crate) struct ResponseGuard(Rc<ClientState>);

#[cfg(any(unix, target_family = "wasm"))]
impl Drop for ResponseGuard {
    fn drop(&mut self) {
        self.0.responses.set(self.0.responses.get() - 1);
        self.0.response_event.notify(usize::MAX);
    }
}

/// A local client of the authoritative runtime. Clones share request idempotency.
/// The host owns `Router::run`; this client never creates an executor or worker thread.
/// Clients retain runtime state; dropping the host driver still stops admission/work.
/// Views can use `PageContext::dispatch` through their weak page navigator.
#[derive(Clone)]
pub struct RuntimeClient<R: Route> {
    router: Router<R>,
    state: Rc<ClientState>,
}

impl<R: Route> RuntimeClient<R> {
    pub(crate) fn new(router: Router<R>, state: Rc<ClientState>) -> Self {
        Self { router, state }
    }

    #[cfg(target_family = "wasm")]
    pub(crate) fn router(&self) -> &Router<R> {
        &self.router
    }

    #[cfg(any(unix, target_family = "wasm"))]
    pub(crate) fn begin_response(&self) -> ResponseGuard {
        self.state.responses.set(self.state.responses.get() + 1);

        ResponseGuard(self.state.clone())
    }

    #[cfg(all(feature = "gpui", not(target_family = "wasm")))]
    pub(crate) async fn responses_idle(&self) {
        loop {
            let listener = self.state.response_event.listen();

            if self.state.responses.get() == 0 {
                return;
            }

            listener.await;
        }
    }

    /// Describes the actual portable capabilities, version and retention limits.
    pub fn info(&self) -> RuntimeInfo {
        let (completed_operations, pending_operations) = self.router.operation_limits();

        RuntimeInfo {
            element_inspection: self.state.inspection.status(),
            element_interaction: self.state.inspection.status()
                != crate::inspection::Status::Unavailable,
            protocol: PROTOCOL_VERSION,
            runtime_id: self.router.runtime_id(),
            platform: if cfg!(target_family = "wasm") {
                "web"
            } else {
                "native"
            }
            .into(),
            commands: Command::names().iter().map(|name| (*name).into()).collect(),
            queries: Query::names().iter().map(|name| (*name).into()).collect(),
            supported_targets: self.router.supported_targets(),
            completed_operations,
            pending_operations,
            idempotency_capacity: self.state.capacity,
        }
    }

    /// Validates protocol/runtime before dispatch. Application mutations require a key.
    /// Leased development highlight/picker commands are transient and do not retain keys.
    /// Keys are retained for this runtime's entire lifetime, including failures.
    /// Exhaustion rejects new keys instead of silently allowing duplicate retries.
    pub fn request(&self, request: Request) -> Result<Reply, ControlError> {
        if request.protocol != PROTOCOL_VERSION {
            return Err(ControlError::ProtocolVersion {
                expected: PROTOCOL_VERSION,
            });
        }

        if request.runtime_id != self.router.runtime_id() {
            return Err(ControlError::WrongRuntime);
        }

        if self.router.is_shutdown() {
            return Err(ControlError::RuntimeShutdown);
        }

        match request.message {
            Message::Query(query) => self.query(query),
            Message::Command(command) => {
                // Renewable development leases must not fill the lifetime mutation-key registry.
                if matches!(
                    command,
                    Command::HighlightElement { .. } | Command::InspectMode { .. }
                ) {
                    return self.command(&command);
                }

                let key = request
                    .key
                    .filter(|key| !key.is_empty() && key.len() <= 128)
                    .ok_or(ControlError::InvalidRequestKey)?;

                if let Some(previous) = self.state.requests.borrow().get(&key) {
                    return if previous.command == command {
                        previous.reply.clone()
                    } else {
                        Err(ControlError::KeyConflict)
                    };
                }

                if self.state.requests.borrow().len() >= self.state.capacity {
                    return Err(ControlError::RequestCapacity);
                }

                let reply = self.command(&command);

                self.state.requests.borrow_mut().insert(
                    key,
                    SavedRequest {
                        command,
                        reply: reply.clone(),
                    },
                );

                reply
            }
        }
    }

    /// Captures an initial snapshot and revision atomically on the local event loop.
    /// `next` checks that revision before awaiting, avoiding a subscription gap.
    pub fn watch(&self) -> RuntimeWatch<R> {
        RuntimeWatch {
            client: self.clone(),
            current: self.router.snapshot(),
        }
    }

    #[cfg(feature = "gpui")]
    pub(crate) fn set_commands(&self, commands: Commands) {
        *self.state.commands.borrow_mut() = commands;
    }

    pub(crate) fn set_services(&self, services: crate::services::ServiceRegistry) {
        *self.state.services.borrow_mut() = services;
    }

    /// Application composition shared with routed views. Registrations on a returned
    /// snapshot do not mutate the running application's service set.
    pub fn services(&self) -> crate::services::ServiceRegistry {
        self.state.services.borrow().clone()
    }

    /// Registers application commands before driving the host. Existing handles share this registry.
    pub fn commands(&self) -> Commands {
        self.state.commands.borrow().clone()
    }

    /// Dispatches typed application arguments through the same wire operation and result registry.
    pub fn dispatch<C: ApplicationCommand>(
        &self,
        command: C,
        key: String,
    ) -> Result<Reply, ControlError> {
        self.request(Request {
            protocol: PROTOCOL_VERSION,
            runtime_id: self.info().runtime_id,
            key: Some(key),
            message: Message::Command(Command::Run {
                id: C::id(),
                arguments: serde_json::to_value(command)
                    .map_err(|error| CommandError::Arguments(error.to_string()))?,
            }),
        })
    }

    fn command(&self, command: &Command) -> Result<Reply, ControlError> {
        let navigator = self.router.navigator();

        let id: OperationId = match command {
            Command::HighlightElement { id, generation } => {
                self.state.inspection.highlight(id.as_deref(), generation)?;
                return Ok(Reply::Acknowledged);
            }
            Command::InspectMode { enabled, lease_ms } => {
                self.state.inspection.inspect_mode(*enabled, *lease_ms)?;
                return Ok(Reply::Acknowledged);
            }

            Command::CloseWindow { window } => self.router.close_window(*window).id(),
            Command::Quit { options } => self.router.quit(*options).id(),
            Command::PrepareReload { request } => {
                return Ok(Reply::Reload(self.router.prepare_reload(request.clone())?));
            }
            Command::FinishReload { id, applied } => {
                return Ok(Reply::Reload(self.router.finish_reload(id, *applied)?));
            }
            Command::Open {
                window,
                destination,
                options,
            } => navigator
                .for_window(window.unwrap_or(navigator.window_id()))
                .open_with(R::decode(destination)?, *options)
                .id(),
            Command::Navigate {
                tab,
                destination,
                replace,
            } => {
                let destination = R::decode(destination)?;
                let tab = navigator.tab(*tab);

                if *replace {
                    tab.replace(destination).id()
                } else {
                    tab.navigate(destination).id()
                }
            }
            Command::Reveal {
                window,
                scope,
                destination,
            } => navigator
                .for_window(window.unwrap_or(navigator.window_id()))
                .reveal(R::decode(destination)?, *scope)
                .id(),
            Command::Select { tab } => navigator.tab(*tab).select().id(),
            Command::Close { tab } => navigator.tab(*tab).close().id(),
            Command::Back { tab } => navigator.tab(*tab).back().id(),
            Command::Forward { tab } => navigator.tab(*tab).forward().id(),
            Command::Mode { tab, mode } => navigator.tab(*tab).set_mode(*mode).id(),
            Command::CancelOperation { operation } => {
                return Ok(Reply::Accepted {
                    accepted: self.router.cancel(*operation)?,
                });
            }
            Command::Respond { decision, choice } => {
                navigator.respond(*decision, choice)?;

                return Ok(Reply::Acknowledged);
            }
            Command::Run { id, arguments } => {
                let commands = self.commands();

                let context = CommandContext {
                    services: self.services(),
                    app: self.router.app(),
                    cancellation: crate::http::CancellationToken::new(),
                };

                let token = context.cancellation.clone();

                let future =
                    commands.prepare(id, arguments.clone(), self.router.has_window(), context)?;

                return Ok(Reply::Submitted {
                    operation_id: self.router.submit_command(id, token, future).id(),
                });
            }
            Command::CancelTask { task } => {
                return Ok(Reply::Accepted {
                    accepted: self.router.cancel_task(*task)?,
                });
            }
        };

        Ok(Reply::Submitted { operation_id: id })
    }

    #[cfg(feature = "gpui")]
    pub(crate) fn inspection(&self) -> crate::inspection::Store {
        self.state.inspection.clone()
    }

    fn query(&self, query: Query) -> Result<Reply, ControlError> {
        match query {
            Query::Windows {} => Ok(Reply::Data(
                serde_json::to_value(self.router.snapshot().windows)
                    .expect("window snapshots serialize"),
            )),
            Query::Window { window } => {
                Ok(Reply::Snapshot(self.router.snapshot_in_window(window)?))
            }
            Query::Quit {} => Ok(Reply::Quit(self.router.quit_status())),
            Query::InspectPick {} => Ok(Reply::InspectPick(self.state.inspection.pick()?)),
            Query::InspectTree {} => Ok(Reply::Elements(self.state.inspection.snapshot())),
            Query::InspectElement { id } => Ok(Reply::Element(self.state.inspection.element(&id)?)),
            Query::Reload {} => Ok(Reply::Reload(self.router.reload_status())),
            Query::Checkpoint { application_id } => Ok(Reply::Checkpoint(
                self.router.reload_checkpoint(&application_id)?,
            )),
            Query::Info {} => Ok(Reply::Info(self.info())),
            Query::Schema {} => Ok(Reply::Data(json!({
                "protocol": PROTOCOL_VERSION,
                "commands": Command::names(), "queries": Query::names(),
                "request": {"protocol": PROTOCOL_VERSION, "runtime_id": self.info().runtime_id,
                    "key": "unique-client-request-key", "message": {"type": "query", "value": {"type": "info"}}},
                "routes": R::descriptors(),
            }))),
            Query::Commands {} => Ok(Reply::Data(
                serde_json::to_value(self.commands().descriptors())
                    .expect("command schema serialization"),
            )),
            Query::Routes {} => Ok(Reply::Data(
                serde_json::to_value(R::descriptors()).expect("route schema serialization"),
            )),
            Query::Snapshot {} => Ok(Reply::Snapshot(self.router.snapshot())),
            Query::Tabs {} => Ok(Reply::Data(
                serde_json::to_value(self.router.snapshot().tabs).expect("tab serialization"),
            )),
            Query::Tasks { owner } => {
                let mut tasks = self.router.snapshot().work.tasks;

                tasks.retain(|task| owner.is_none_or(|owner| owner == task.owner));

                Ok(Reply::Data(
                    serde_json::to_value(tasks).expect("task serialization"),
                ))
            }
            Query::Operations {} => Ok(Reply::Data(
                serde_json::to_value(self.router.snapshot().operations)
                    .expect("status serialization"),
            )),
            Query::Operation { operation } => Ok(Reply::Operation(OperationRecord {
                operation_id: operation,
                state: self.router.operation(operation)?,
                result: self.router.operation_result(operation),
            })),
            Query::Decisions {} => Ok(Reply::Data(
                serde_json::to_value(self.router.snapshot().decisions)
                    .expect("decision serialization"),
            )),
            Query::Events { cursor } => Ok(Reply::Event(event(self.router.snapshot(), cursor))),
            Query::Diagnostics { after, kind } => {
                let mut snapshot = self.router.diagnostics();

                snapshot.records.retain(|record| {
                    record.sequence > after
                        && kind.as_ref().is_none_or(|kind| {
                            if kind == "span" {
                                record.kind == "span_open" || record.kind == "span_close"
                            } else {
                                record.kind == *kind
                            }
                        })
                });

                Ok(Reply::Diagnostics(snapshot))
            }
        }
    }
}

fn event(snapshot: crate::router::RouterSnapshot, previous: Option<EventCursor>) -> RuntimeEvent {
    let cursor = EventCursor {
        runtime_id: snapshot.runtime_id,
        epoch: 1,
        revision: snapshot.revision,
    };

    let kind = match previous {
        None => EventKind::Snapshot,
        Some(previous) if previous == cursor => EventKind::Unchanged,
        Some(previous)
            if previous.runtime_id == cursor.runtime_id
                && previous.epoch == cursor.epoch
                && previous.revision.checked_add(1) == Some(cursor.revision) =>
        {
            EventKind::Changed
        }
        Some(_) => EventKind::Gap,
    };

    RuntimeEvent {
        cursor,
        kind,
        snapshot: if kind == EventKind::Unchanged {
            None
        } else {
            Some(snapshot)
        },
    }
}

/// Revisioned local observation. State notifications can coalesce; a skipped revision
/// is an explicit `Gap` event carrying a new authoritative snapshot, never inferred logs.
pub struct RuntimeWatch<R: Route> {
    client: RuntimeClient<R>,
    current: crate::router::RouterSnapshot,
}

impl<R: Route> RuntimeWatch<R> {
    /// The last authoritative snapshot, initially captured when the watch was created.
    pub fn snapshot(&self) -> &crate::router::RouterSnapshot {
        &self.current
    }

    /// Waits for a state change. Dropping the wait never cancels runtime operations.
    pub async fn next(&mut self) -> Result<RuntimeEvent, ControlError> {
        if self.client.router.is_shutdown() {
            return Err(ControlError::RuntimeShutdown);
        }

        self.client.router.changed(self.current.revision).await;
        let previous = EventCursor {
            runtime_id: self.current.runtime_id,
            epoch: 1,
            revision: self.current.revision,
        };

        self.current = self.client.router.snapshot();

        Ok(event(self.current.clone(), Some(previous)))
    }
}

/// Runner development endpoint policy. Release and debug builds both default to Disabled.
#[derive(Clone, Copy, Debug, Default)]
pub enum ControlConfig {
    /// No listener, discovery files or outbound bridge.
    #[default]
    Disabled,
    /// Native requires `RAKE_DEV_CONTROL=1`; Web requires injected same-origin metadata.
    FromEnvironment,
}

#[cfg(feature = "gpui")]
impl ControlConfig {
    pub(crate) fn inspection_enabled(self) -> bool {
        if !matches!(self, Self::FromEnvironment) {
            return false;
        }

        #[cfg(unix)]
        {
            std::env::var("RAKE_DEV_CONTROL").as_deref() == Ok("1")
        }
        #[cfg(target_family = "wasm")]
        {
            WebBridgeConfig::from_document().is_ok_and(|config| config.is_some())
        }

        #[cfg(not(any(unix, target_family = "wasm")))]
        {
            false
        }
    }
}

#[cfg(feature = "gpui")]
pub(crate) async fn run_host<R: Route>(
    config: ControlConfig,
    client: RuntimeClient<R>,
    application_id: String,
) {
    if matches!(config, ControlConfig::FromEnvironment) {
        #[cfg(unix)]
        if std::env::var("RAKE_DEV_CONTROL").as_deref() == Ok("1") {
            match local::LocalServer::bind(&client, &application_id, &local::directory()) {
                Ok(server) => {
                    if let Err(error) = server.run(client).await {
                        crate::error!(%error, "Development control stopped");
                    }
                }
                Err(error) => crate::error!(%error, "Development control could not start"),
            }
        }

        #[cfg(target_family = "wasm")]
        match WebBridgeConfig::from_document() {
            Ok(Some(config)) => match WebBridge::new(config) {
                Ok(bridge) => bridge.run(client, &application_id).await,
                Err(error) => crate::error!(%error, "Development bridge could not start"),
            },
            Ok(None) => {}
            Err(error) => crate::error!(%error, "Invalid development bridge configuration"),
        }
    }

    #[cfg(not(any(unix, target_family = "wasm")))]
    let _ = (client, application_id);

    futures::future::pending::<()>().await;
}

#[cfg(test)]
mod inspection_control_tests {
    //! Covers: leased commands bypass operation/key retention while preserving runtime validation.
    //! Real: shared client dispatcher and inspection store. Doubles: an empty captured host.
    use super::*;
    use crate::router::{NoRoute, RouterConfig, WindowId};

    #[test]
    fn development_lease_renewals_never_fill_application_retry_capacity() {
        let router = Router::<NoRoute>::new(RouterConfig::default());
        let client = router.client();
        assert!(!client.info().element_interaction);
        let snapshot = client.state.inspection.begin(0);
        client.state.inspection.publish(WindowId(1), snapshot);
        for _ in 0..2048 {
            let reply = client
                .request(Request {
                    protocol: PROTOCOL_VERSION,
                    runtime_id: router.runtime_id(),
                    key: None,
                    message: Message::Command(Command::InspectMode {
                        enabled: true,
                        lease_ms: 1000,
                    }),
                })
                .unwrap();
            assert!(matches!(reply, Reply::Acknowledged));
        }
        assert!(client.state.requests.borrow().is_empty());
        assert!(router.snapshot().operations.is_empty());
        assert!(client.info().element_interaction);
    }
}
