//! Bounded, owned inspection of the presented view tree.
//!
//! GPUI's public API does not enumerate arbitrary descendants. Rake captures its
//! shell and application-supplied inspection boundaries, preserving their nearest
//! inspected ancestor. Layout is measured by GPUI; styles are authored base
//! refinements, not computed/hover styles. No views or source contents are retained.

#[cfg(feature = "gpui")]
mod view;

#[cfg(feature = "gpui")]
pub use view::{InspectElement, Inspected, inspect};

use std::cell::RefCell;
use std::rc::Rc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::router::{EntryId, TabId};

/// Maximum nodes in a captured tree. Remaining nodes are counted as omitted.
pub const MAX_ELEMENTS: usize = 256;
/// Maximum serialized snapshot size, below the transport's 1 MiB limit.
pub const MAX_SNAPSHOT_BYTES: usize = 512 * 1024;

/// Window-local logical pixels, after layout and element offsets; not screen pixels.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Layout {
    /// Horizontal origin.
    pub x: f32,
    /// Vertical origin.
    pub y: f32,
    /// Laid-out width before clipping.
    pub width: f32,
    /// Laid-out height before clipping.
    pub height: f32,
}

/// Construction site of an inspection boundary; no file access is provided.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceLocation {
    /// Compiler-supplied source path, possibly relative or remapped.
    pub file: String,
    /// One-based line.
    pub line: u32,
    /// One-based column.
    pub column: u32,
}

/// One boundary in prepaint traversal order. Semantic IDs may repeat in a tree.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Element {
    /// Opaque identity scoped to the runtime and this capture; never a semantic selector.
    pub id: String,
    /// Nearest retained inspected ancestor; uninstrumented intermediates are omitted.
    pub parent: Option<String>,
    /// Rust element type, without a promise of a stable wire spelling.
    pub kind: String,
    /// Application-supplied identity, distinct from GPUI state identity.
    pub semantic_id: Option<String>,
    /// Construction site of the boundary, when supplied.
    pub source: Option<SourceLocation>,
    /// Actual GPUI layout, or None for invalid/non-finite geometry.
    pub layout: Option<Layout>,
    /// Serialized GPUI base style refinement; None for custom unstyled elements.
    pub styles: Option<Value>,
    /// Whether a metadata field exceeded its capture limit.
    pub metadata_truncated: bool,
    /// Presented route owner; None for the workspace shell.
    pub tab_id: Option<TabId>,
    /// Presented route entry, never a hidden retained history entry.
    pub entry_id: Option<EntryId>,
}

/// Coverage and availability, independent of whether the current tree is empty.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// No mounted inspection host (including headless runtimes).
    #[default]
    Unavailable,
    /// Attached host has not completed a capture yet.
    Waiting,
    /// A completed prepaint traversal is available.
    Captured,
}

/// Latest completed tree. Replacing a capture invalidates every previous element ID.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    /// Explicit availability.
    pub status: Status,
    /// Monotonic capture identity, encoded as a string for JavaScript precision.
    pub capture: String,
    /// Router revision at the start of the captured render, not a live revision.
    pub router_revision: u64,
    /// Captured boundaries in depth-first order.
    pub elements: Vec<Element>,
    /// Inspected boundaries omitted by count/byte limits. Opaque descendants are uncounted.
    pub omitted: usize,
    /// Human-readable coverage including opaque/cached subtrees and style limitations.
    pub coverage: String,
}

/// Exact lookup failure. Clients refresh the tree after stale captures.
#[derive(Clone, Debug, thiserror::Error, Serialize, Deserialize)]
pub enum InspectionError {
    /// No suitable mounted host.
    #[error("element inspection is unavailable for this runtime")]
    Unavailable,
    /// Host is waiting for its first completed capture.
    #[error("element inspection is waiting for a completed frame")]
    Waiting,
    /// ID is invalid, omitted, or belongs to a replaced capture.
    #[error("element identity is not retained; refresh the inspected tree")]
    NotRetained,
}

#[derive(Clone)]
pub(crate) struct Store(Rc<RefCell<Snapshot>>);

impl Default for Store {
    fn default() -> Self {
        Self(Rc::new(RefCell::new(Snapshot {
            status: Status::Unavailable,
            capture: "0".into(),
            router_revision: 0,
            elements: Vec::new(),
            omitted: 0,
            coverage: "Rake inspection boundaries only; opaque GPUI descendants and reused cached subtrees are not enumerated. Layout is before clipping; styles are authored base refinements, not computed, inherited or interaction styles. Text content is not captured.".into(),
        })))
    }
}

impl Store {
    pub(crate) fn snapshot(&self) -> Snapshot {
        self.0.borrow().clone()
    }

    pub(crate) fn element(&self, id: &str) -> Result<Element, InspectionError> {
        let snapshot = self.0.borrow();

        match snapshot.status {
            Status::Unavailable => Err(InspectionError::Unavailable),
            Status::Waiting => Err(InspectionError::Waiting),
            Status::Captured => snapshot
                .elements
                .iter()
                .find(|element| element.id == id)
                .cloned()
                .ok_or(InspectionError::NotRetained),
        }
    }

    pub(crate) fn status(&self) -> Status {
        self.0.borrow().status
    }

    #[cfg(feature = "gpui")]
    pub(crate) fn begin(&self, revision: u64) -> Snapshot {
        let snapshot = self.0.borrow();

        Snapshot {
            capture: snapshot
                .capture
                .parse::<u64>()
                .expect("internal capture identity")
                .checked_add(1)
                .expect("inspection capture overflow")
                .to_string(),
            status: Status::Captured,
            router_revision: revision,
            elements: Vec::new(),
            omitted: 0,
            coverage: snapshot.coverage.clone(),
        }
    }

    #[cfg(feature = "gpui")]
    pub(crate) fn attach(&self) {
        self.0.borrow_mut().status = Status::Waiting;
    }

    #[cfg(feature = "gpui")]
    pub(crate) fn publish(&self, snapshot: Snapshot) {
        *self.0.borrow_mut() = snapshot;
    }
}

#[cfg(any(feature = "gpui", test))]
impl Snapshot {
    // Leaves room for envelope, coverage and JSON separators below the wire limit.
    fn retain(&mut self, element: Element, bytes: &mut usize) -> Option<String> {
        let size = serde_json::to_vec(&element).map_or(MAX_SNAPSHOT_BYTES, |value| value.len());

        if self.elements.len() >= MAX_ELEMENTS || *bytes + size >= MAX_SNAPSHOT_BYTES - 4096 {
            self.omitted += 1;

            return None;
        }

        *bytes += size;
        let id = element.id.clone();

        self.elements.push(element);

        Some(id)
    }
}

#[cfg(test)]
mod tests;
