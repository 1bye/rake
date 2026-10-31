//! Bounded development projections of explicitly registered typed stores.

use serde::{Deserialize, Serialize};

/// Chrome storage presentation. The backend remains authoritative for persistence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Area {
    /// Application data intended to persist across launches.
    Local,
    /// Application data intended to last only for this session.
    Session,
}

/// Public inspection metadata; no values are included in catalog polling.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Descriptor {
    /// Unique stable registration name.
    pub name: String,
    /// Chrome's local/session storage section.
    pub area: Area,
    /// Whether DevTools is allowed to write/remove values.
    pub writable: bool,
    /// Process-local change revision shared by cloned store handles.
    pub revision: u64,
}

/// Read-only storage request.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Read {
    /// Catalog registered stores without accessing their backends.
    Stores,
    /// Exclusive cursor page of typed JSON values.
    Entries {
        /// Exact registration name.
        store: String,
        /// Exclusive key cursor.
        after: Option<String>,
        /// 1–256 keys, further bounded by encoded response bytes.
        limit: usize,
    },
}

/// A persistent application mutation, validated by the store's value type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    /// Exact writable registration name.
    pub store: String,
    /// Exact backend key.
    pub key: String,
    /// JSON text to decode and store; `None` removes the key.
    pub value: Option<String>,
}

/// A bounded owned key/value page. This is not a cross-page transaction.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Page {
    /// Registration revision observed before/after this page.
    pub revision: u64,
    /// Key and canonical typed JSON text pairs.
    pub entries: Vec<(String, String)>,
    /// Exclusive next cursor, or `None` when finished.
    pub next: Option<String>,
}
