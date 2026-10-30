//! Explicit development-only access to application storage. No automatic discovery.
//!
//! Registering a store exposes its decrypted values. Keep credentials in separate,
//! unregistered stores. Writes persist through the same typed/backend/cache path
//! as application calls, and are not temporary Elements overrides.

use super::{MAX_VALUE_BYTES, StorageError, Store};
use futures::future::LocalBoxFuture;
use rake_protocol::control::{Command, ControlError, Message, Query, Reply, Request};
pub use rake_protocol::storage::{Area, Descriptor, Page};
use rake_protocol::storage::{Edit, Read};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
};

const PAGE_BYTES: usize = 384 * 1024;
const MAX_STORES: usize = 32;
const REQUESTS: usize = 256;
const REQUEST_BYTES: usize = 2 * 1024 * 1024;

trait Inspect {
    fn revision(&self) -> u64;
    fn page<'a>(
        &'a self,
        after: Option<&'a str>,
        limit: usize,
    ) -> LocalBoxFuture<'a, Result<Page, StorageError>>;
    fn edit<'a>(&'a self, edit: &'a Edit) -> LocalBoxFuture<'a, Result<(), StorageError>>;
}

impl<T: Serialize + DeserializeOwned + 'static> Inspect for Store<T> {
    fn revision(&self) -> u64 {
        self.inner.revision.get()
    }
    fn page<'a>(
        &'a self,
        after: Option<&'a str>,
        limit: usize,
    ) -> LocalBoxFuture<'a, Result<Page, StorageError>> {
        Box::pin(async move {
            let revision = self.revision();
            let keys = self.keys(after, limit).await?;
            let mut entries = Vec::new();
            let mut bytes = 0;
            let mut next = None;
            for key in &keys {
                if let Some(value) = self.get(key).await? {
                    let value = serde_json::to_string(&value)?;
                    super::validate_value(value.as_bytes())?;
                    if bytes + key.len() + value.len() > PAGE_BYTES {
                        next = entries
                            .last()
                            .map(|(key, _): &(String, String)| key.clone());
                        break;
                    }
                    bytes += key.len() + value.len();
                    entries.push((key.clone(), value));
                }
            }
            if revision != self.revision() {
                return Err(StorageError::Inspection(
                    "store changed while reading; refresh".into(),
                ));
            }
            if next.is_none() && keys.len() == limit {
                next = keys.last().cloned();
            }
            Ok(Page {
                revision,
                entries,
                next,
            })
        })
    }
    fn edit<'a>(&'a self, edit: &'a Edit) -> LocalBoxFuture<'a, Result<(), StorageError>> {
        Box::pin(async move {
            match &edit.value {
                Some(value) => {
                    if value.len() > MAX_VALUE_BYTES {
                        return Err(StorageError::ValueTooLarge);
                    }
                    let value: T = serde_json::from_str(value)?;
                    self.set(&edit.key, &value).await
                }
                None => self.remove(&edit.key).await,
            }
        })
    }
}

struct Registered {
    area: Area,
    writable: bool,
    store: Rc<dyn Inspect>,
}
struct Saved {
    edit: Edit,
    reply: Option<Result<Reply, ControlError>>,
}

/// Immutable registration snapshots sharing their stores and mutation retry ledger.
/// At most 32 named stores and 256 persistent edit request keys per registry.
#[derive(Clone, Default)]
pub struct Storage {
    stores: Rc<BTreeMap<String, Rc<Registered>>>,
    requests: Rc<RefCell<BTreeMap<String, Saved>>>,
    request_bytes: Rc<Cell<usize>>,
}

impl std::fmt::Debug for Storage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Storage")
            .field("names", &self.stores.keys())
            .finish()
    }
}

impl Storage {
    /// Creates an empty registry. Unregistered stores are never discovered.
    pub fn new() -> Self {
        Self::default()
    }
    /// Exposes typed values for reading only. Names contain 1–64 ASCII letters,
    /// digits, dots, underscores or hyphens; duplicate names fail explicitly.
    pub fn read_only<T: Serialize + DeserializeOwned + 'static>(
        self,
        name: &str,
        area: Area,
        store: Store<T>,
    ) -> Result<Self, StorageError> {
        self.register(name, area, false, store)
    }
    /// Exposes persistent typed set/remove operations in addition to reads.
    /// The value's Serde decoder and backend remain authoritative validators.
    pub fn read_write<T: Serialize + DeserializeOwned + 'static>(
        self,
        name: &str,
        area: Area,
        store: Store<T>,
    ) -> Result<Self, StorageError> {
        self.register(name, area, true, store)
    }
    fn register<T: Serialize + DeserializeOwned + 'static>(
        mut self,
        name: &str,
        area: Area,
        writable: bool,
        store: Store<T>,
    ) -> Result<Self, StorageError> {
        if name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            || self.stores.contains_key(name)
            || self.stores.len() >= MAX_STORES
        {
            return Err(StorageError::Inspection(
                "invalid/duplicate storage registration or 32-store limit".into(),
            ));
        }
        Rc::make_mut(&mut self.stores).insert(
            name.into(),
            Rc::new(Registered {
                area,
                writable,
                store: Rc::new(store),
            }),
        );
        Ok(self)
    }
    /// Captures names, permissions and change revisions without reading values.
    pub fn catalog(&self) -> Vec<Descriptor> {
        self.stores
            .iter()
            .map(|(name, registered)| Descriptor {
                name: name.clone(),
                area: registered.area,
                writable: registered.writable,
                revision: registered.store.revision(),
            })
            .collect()
    }
    pub(crate) fn accepts(request: &Request) -> bool {
        matches!(
            request.message,
            Message::Query(Query::Storage { .. }) | Message::Command(Command::EditStorage { .. })
        )
    }
    pub(crate) async fn request(&self, request: Request) -> Result<Reply, ControlError> {
        let failure = |error: StorageError| ControlError::Storage {
            reason: error.to_string(),
        };
        match request.message {
            Message::Query(Query::Storage { read: Read::Stores }) => Ok(Reply::Data(
                serde_json::to_value(self.catalog())
                    .map_err(StorageError::from)
                    .map_err(failure)?,
            )),
            Message::Query(Query::Storage {
                read:
                    Read::Entries {
                        store,
                        after,
                        limit,
                    },
            }) => {
                let registered = self
                    .stores
                    .get(&store)
                    .ok_or_else(|| ControlError::Storage {
                        reason: "unknown storage registration".into(),
                    })?;
                let page = registered
                    .store
                    .page(after.as_deref(), limit)
                    .await
                    .map_err(failure)?;
                Ok(Reply::Data(
                    serde_json::to_value(page)
                        .map_err(StorageError::from)
                        .map_err(failure)?,
                ))
            }
            Message::Command(Command::EditStorage { edit }) => {
                super::validate_key(&edit.key).map_err(failure)?;
                if edit.store.len() > 64 {
                    return Err(ControlError::Storage {
                        reason: "invalid storage registration name".into(),
                    });
                }
                if let Some(value) = &edit.value {
                    super::validate_value(value.as_bytes()).map_err(failure)?;
                }
                let key = request
                    .key
                    .filter(|key| !key.is_empty() && key.len() <= 128)
                    .ok_or(ControlError::InvalidRequestKey)?;
                {
                    let requests = self.requests.borrow();
                    if let Some(saved) = requests.get(&key) {
                        if saved.edit != edit {
                            return Err(ControlError::KeyConflict);
                        }
                        return saved.reply.clone().unwrap_or_else(|| Err(ControlError::Storage { reason: "edit still pending or its response was cancelled; do not resubmit with a new key".into() }));
                    }
                    let bytes = key.len()
                        + edit.store.len()
                        + edit.key.len()
                        + edit.value.as_ref().map_or(0, String::len);
                    if requests.len() >= REQUESTS
                        || self.request_bytes.get() + bytes > REQUEST_BYTES
                    {
                        return Err(ControlError::RequestCapacity);
                    }
                    self.request_bytes.set(self.request_bytes.get() + bytes);
                }
                self.requests.borrow_mut().insert(
                    key.clone(),
                    Saved {
                        edit: edit.clone(),
                        reply: None,
                    },
                );
                let reply = match self.stores.get(&edit.store) {
                    Some(registered) if registered.writable => registered
                        .store
                        .edit(&edit)
                        .await
                        .map(|()| Reply::Acknowledged)
                        .map_err(failure),
                    _ => Err(ControlError::Storage {
                        reason: "unknown or read-only storage registration".into(),
                    }),
                };
                if let Some(saved) = self.requests.borrow_mut().get_mut(&key) {
                    saved.reply = Some(reply.clone());
                }
                reply
            }
            _ => Err(ControlError::Unsupported {
                operation: "storage request".into(),
            }),
        }
    }
}
