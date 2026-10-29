//! Bounded owned element metadata, without renderer dependencies.

use crate::router::{EntryId, TabId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Shared presentation of captured style values.
pub mod styles;

/// Maximum nodes in a captured tree. Remaining nodes are counted as omitted.
pub const MAX_ELEMENTS: usize = 256;
/// Maximum serialized snapshot size, below the transport's 1 MiB limit.
pub const MAX_SNAPSHOT_BYTES: usize = 512 * 1024;
/// Maximum serialized resolved style per boundary, within the total snapshot budget.
pub const MAX_RESOLVED_STYLE_BYTES: usize = 16 * 1024;

/// Window-local logical pixels, after layout and element offsets; not screen pixels.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SourceLocation {
    /// Compiler-supplied source path, possibly relative or remapped.
    pub file: String,
    /// One-based line.
    pub line: u32,
    /// One-based column.
    pub column: u32,
}

/// GPUI's current visual style sampled during paint, including inherited text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResolvedStyle {
    /// Complete resolved GPUI style; relative layout lengths retain their units.
    pub properties: Value,
    /// Whether GPUI's actual hitbox is hovered in this frame.
    pub hovered: bool,
}

/// One boundary in prepaint traversal order. Semantic IDs may repeat in a tree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Element {
    /// Opaque identity scoped to the runtime and this capture; never a semantic selector.
    pub id: String,
    /// Opaque presentation identity across consecutive captures of the same boundary.
    /// Absent for ambiguous repeated boundaries and older runtimes. Never a control ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stable_id: Option<String>,
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
    /// Visible content-mask intersection in window-local logical pixels.
    #[serde(default)]
    pub content_mask: Option<Layout>,
    /// Serialized GPUI base style refinement; None for custom unstyled elements.
    pub styles: Option<Value>,
    /// Live resolved style for GPUI `Div`/`Stateful<Div>`; absent for opaque/custom elements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<ResolvedStyle>,
    /// Whether a metadata field exceeded its capture limit.
    pub metadata_truncated: bool,
    /// Presented route owner; None for the workspace shell.
    pub tab_id: Option<TabId>,
    /// Presented route entry, never a hidden retained history entry.
    pub entry_id: Option<EntryId>,
}

/// Short inspection label without Rust module paths, generic arguments or GPUI wrappers.
/// The captured [`Element::kind`] retains the complete Rust type.
///
/// ```
/// assert_eq!(rake_protocol::inspection::display_name("gpui::Stateful<gpui::Div>"), "Div");
/// assert_eq!(rake_protocol::inspection::display_name("gpui::ViewElement<app::TabBar<app::Route>>"), "TabBar");
/// ```
#[must_use]
pub fn display_name(mut kind: &str) -> &str {
    while let Some((wrapper, inner)) = kind.split_once('<') {
        if matches!(
            wrapper.rsplit("::").next(),
            Some("Stateful" | "ViewElement")
        ) && let Some(inner) = inner.strip_suffix('>')
        {
            kind = inner;
        } else {
            break;
        }
    }

    kind.split('<')
        .next()
        .unwrap_or(kind)
        .rsplit("::")
        .next()
        .unwrap_or(kind)
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

/// Latest completed tree. Capture IDs expire on replacement; presentation identities may persist.
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
    /// A command referred to a replaced capture generation.
    #[error("inspection generation is stale; refresh the capture")]
    StaleGeneration,
    /// Inspect-mode leases must be finite and at most two seconds.
    #[error("inspection lease must be 1–2000 milliseconds")]
    InvalidLease,
}

/// Bounded picker status. Clients deduplicate clicks using `pick_sequence`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Pick {
    /// Whether the input-capture lease remains active.
    pub mode: bool,
    /// Capture identity of the reported hover/pick, not a router revision.
    pub generation: String,
    /// Boundary under the latest pointer observation.
    pub hovered: Option<String>,
    /// Most recent clicked boundary; retained until the next mode begins.
    pub picked: Option<String>,
    /// Increases once per click; polling never consumes another client's pick.
    pub pick_sequence: u64,
}
