//! Inspection-only runtime identity and dispatch for an application-owned root view.

use std::{cell::Cell, rc::Rc};

use super::{
    Command, ControlError, HostClient, Message, PROTOCOL_VERSION, Query, Reply, Request,
    RuntimeInfo,
};
use crate::{
    diagnostics::Collector,
    inspection::{Status, Store},
    router::{RuntimeId, WindowId},
};

#[derive(Clone)]
pub(crate) struct InspectionClient(Rc<State>);

struct State {
    id: RuntimeId,
    inspection: Store,
    collector: Collector,
    closed: Cell<bool>,
}

impl InspectionClient {
    pub(crate) fn new(collector: Collector) -> Self {
        let inspection = Store::default();
        inspection.attach();
        Self(Rc::new(State {
            id: RuntimeId(uuid::Uuid::new_v4()),
            inspection,
            collector,
            closed: Cell::new(false),
        }))
    }

    pub(crate) fn inspection(&self) -> Store {
        self.0.inspection.clone()
    }

    pub(crate) fn host(&self) -> HostClient {
        HostClient::new(self.clone())
    }

    pub(crate) fn close(&self) {
        self.0.closed.set(true);
        let _ = self.0.inspection.inspect_mode(false, 0);
        let _ = self.0.inspection.highlight(None, "");
        self.0.inspection.detach_window(WindowId(1));
    }
}

impl super::host::Host for InspectionClient {
    fn info(&self) -> RuntimeInfo {
        let status = if self.0.closed.get() {
            Status::Unavailable
        } else {
            self.0.inspection.status()
        };
        RuntimeInfo {
            protocol: PROTOCOL_VERSION,
            runtime_id: self.0.id,
            platform: if cfg!(target_family = "wasm") {
                "web"
            } else {
                "native"
            }
            .into(),
            element_inspection: status,
            element_interaction: status != Status::Unavailable,
            commands: [
                "navigate_element",
                "highlight_element",
                "inspect_mode",
                "edit_element_style",
            ]
            .map(str::to_owned)
            .into(),
            queries: [
                "info",
                "schema",
                "diagnostics",
                "inspect_tree",
                "inspect_tree_page",
                "inspect_element",
                "inspect_pick",
            ]
            .map(str::to_owned)
            .into(),
            supported_targets: Vec::new(),
            completed_operations: 0,
            pending_operations: 0,
            idempotency_capacity: 0,
        }
    }

    fn request(&self, request: Request) -> Result<Reply, ControlError> {
        if request.protocol != PROTOCOL_VERSION {
            return Err(ControlError::ProtocolVersion {
                expected: PROTOCOL_VERSION,
            });
        }
        if request.runtime_id != self.0.id {
            return Err(ControlError::WrongRuntime);
        }
        if self.0.closed.get() {
            return Err(ControlError::RuntimeShutdown);
        }
        let store = &self.0.inspection;
        match request.message {
            Message::Query(Query::Info {}) => Ok(Reply::Info(self.info())),
            Message::Query(Query::Schema {}) => Ok(Reply::Data(
                serde_json::json!({"protocol":PROTOCOL_VERSION,"runtime_id":self.0.id,"commands":self.info().commands,"queries":self.info().queries}),
            )),
            Message::Query(Query::Diagnostics { after, kind }) => Ok(Reply::Diagnostics(
                self.0.collector.snapshot_after(after, kind.as_deref()),
            )),
            Message::Query(Query::InspectTreePage { generation, offset }) => Ok(
                Reply::ElementPage(store.tree_page(generation.as_deref(), offset)?),
            ),
            Message::Query(Query::InspectTree {}) => Ok(Reply::Elements(store.snapshot())),
            Message::Query(Query::InspectElement { id }) => Ok(Reply::Element(store.element(&id)?)),
            Message::Query(Query::InspectPick {}) => Ok(Reply::InspectPick(store.pick()?)),
            Message::Command(Command::HighlightElement { id, generation }) => {
                store.highlight(id.as_deref(), &generation)?;
                Ok(Reply::Acknowledged)
            }
            Message::Command(Command::InspectMode { enabled, lease_ms }) => {
                store.inspect_mode(enabled, lease_ms)?;
                Ok(Reply::Acknowledged)
            }
            Message::Command(Command::EditElementStyle {
                id,
                generation,
                edit,
            }) => {
                store.edit_style(&id, &generation, &edit)?;
                Ok(Reply::Acknowledged)
            }
            Message::Command(Command::NavigateElement {
                id,
                generation,
                action,
            }) => {
                store.navigate(&id, &generation, action)?;
                Ok(Reply::Acknowledged)
            }
            _ => Err(ControlError::Unsupported {
                operation: "router state and application commands on a standalone inspection host"
                    .into(),
            }),
        }
    }

    #[cfg(target_family = "wasm")]
    fn reload<'a>(
        &'a self,
        bridge: &'a super::WebBridge,
        _application_id: &'a str,
        build: u64,
    ) -> futures::future::LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(bridge.reload_standalone(self.0.id, build))
    }
}

#[cfg(test)]
mod tests;
