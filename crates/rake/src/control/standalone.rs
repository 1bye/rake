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
    storage: std::cell::RefCell<crate::storage::devtools::Storage>,
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
            storage: std::cell::RefCell::new(crate::storage::devtools::Storage::new()),
        }))
    }

    pub(crate) fn inspection(&self) -> Store {
        self.0.inspection.clone()
    }

    pub(crate) fn set_storage(&self, storage: crate::storage::devtools::Storage) {
        *self.0.storage.borrow_mut() = storage;
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
    fn storage(&self) -> crate::storage::devtools::Storage {
        self.0.storage.borrow().clone()
    }
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
                "preview_input",
                "edit_storage",
                "record_memory",
                "navigate_element",
                "highlight_element",
                "inspect_mode",
                "edit_element_style",
                "edit_element_text",
                "record_frames",
            ]
            .into_iter()
            .filter(|name| *name != "record_memory" || super::memory::available())
            .filter(|name| *name != "preview_input" || self.0.inspection.preview_enabled())
            .map(str::to_owned)
            .collect(),
            queries: [
                "preview",
                "preview_element",
                "storage",
                "memory",
                "info",
                "schema",
                "diagnostics",
                "http_body",
                "inspect_tree",
                "inspect_revision",
                "inspect_window",
                "inspect_tree_page",
                "frames",
                "inspect_element",
                "inspect_pick",
            ]
            .into_iter()
            .filter(|name| *name != "memory" || super::memory::available())
            .filter(|name| !name.starts_with("preview") || self.0.inspection.preview_enabled())
            .map(str::to_owned)
            .collect(),
            supported_targets: Vec::new(),
            completed_operations: 0,
            pending_operations: 0,
            idempotency_capacity: crate::storage::devtools::REQUESTS,
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
            Message::Query(Query::HttpBody {
                request_id,
                response,
            }) => Ok(Reply::Data(
                serde_json::to_value(self.0.collector.http_body(request_id, response))
                    .unwrap_or_default(),
            )),
            Message::Query(Query::Diagnostics { after, kind }) => Ok(Reply::Diagnostics(
                self.0.collector.snapshot_after(after, kind.as_deref()),
            )),
            Message::Query(Query::InspectTreePage { generation, offset }) => Ok(
                Reply::ElementPage(store.tree_page(generation.as_deref(), offset)?),
            ),
            Message::Query(Query::Frames { after }) => {
                Ok(Reply::Frames(store.frames(after, &self.0.collector)))
            }
            Message::Query(Query::Memory {
                owner,
                include_freed,
            }) => super::memory::snapshot(&owner, include_freed),
            Message::Command(Command::RecordMemory {
                owner,
                enabled,
                interval,
                lease_ms,
            }) => super::memory::record(&owner, enabled, interval, lease_ms),
            Message::Query(Query::InspectTree {}) => Ok(Reply::Elements(store.snapshot())),
            Message::Query(Query::InspectWindow {
                window,
                presentation,
                offset,
            }) => Ok(Reply::InspectionWindow(store.window_page(
                window,
                presentation.as_deref(),
                offset,
            )?)),
            Message::Query(Query::InspectRevision {}) => {
                Ok(Reply::InspectionRevision(store.tree_revision()))
            }
            Message::Query(Query::InspectElement { id }) => Ok(Reply::Element(store.element(&id)?)),
            Message::Query(Query::InspectPick {}) => Ok(Reply::InspectPick(store.pick()?)),
            Message::Query(Query::Preview {}) => Ok(Reply::Data(
                serde_json::to_value(store.preview_windows()).expect("preview windows serialize"),
            )),
            Message::Query(Query::PreviewElement { point }) => {
                Ok(Reply::Element(store.preview_element(&point)?))
            }
            Message::Command(Command::PreviewInput { point, input }) => {
                store.preview_input(&point, &input)?;

                Ok(Reply::Acknowledged)
            }
            Message::Command(Command::HighlightElement { id, generation }) => {
                store.highlight(id.as_deref(), &generation)?;
                Ok(Reply::Acknowledged)
            }
            Message::Command(Command::InspectMode { enabled, lease_ms }) => {
                store.inspect_mode(enabled, lease_ms)?;
                Ok(Reply::Acknowledged)
            }
            Message::Command(Command::RecordFrames { enabled, lease_ms }) => {
                store.record_frames(enabled, lease_ms)?;
                Ok(Reply::Acknowledged)
            }
            Message::Command(Command::EditElementText {
                id,
                generation,
                edit,
            }) => {
                store.edit_text(&id, &generation, &edit)?;
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
mod storage_tests {
    use super::*;
    use crate::storage::{
        MemoryBackend, Store as TypedStore,
        devtools::{Area, Storage},
    };
    use rake_protocol::storage::{Edit, Read};

    #[test]
    fn standalone_storage_uses_async_dispatch_and_closes_with_its_host() {
        futures::executor::block_on(async {
            let store = TypedStore::<String>::new(MemoryBackend::new());
            let client = InspectionClient::new(Collector::default());
            client.set_storage(
                Storage::new()
                    .read_write("settings", Area::Local, store.clone())
                    .unwrap(),
            );
            let host = client.host();
            let request = Request {
                protocol: PROTOCOL_VERSION,
                runtime_id: host.info().runtime_id,
                key: Some("write".into()),
                message: Message::Command(Command::EditStorage {
                    edit: Edit {
                        store: "settings".into(),
                        key: "theme".into(),
                        value: Some("\"dark\"".into()),
                    },
                }),
            };
            assert!(matches!(
                host.request_async(request.clone()).await.unwrap(),
                Reply::Acknowledged
            ));
            assert_eq!(store.get("theme").await.unwrap().as_deref(), Some("dark"));
            let read = Request {
                key: None,
                message: Message::Query(Query::Storage { read: Read::Stores }),
                ..request.clone()
            };
            assert!(matches!(
                host.request_async(read).await.unwrap(),
                Reply::Data(_)
            ));
            client.close();
            assert!(matches!(
                host.request_async(request).await,
                Err(ControlError::RuntimeShutdown)
            ));
        });
    }
}

#[cfg(test)]
mod tests;
