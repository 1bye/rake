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

/// Unclipped box geometry in window-local logical pixels. Missing boxes are unknown,
/// never substitutes for the border box. Absolute edges use GPUI's device-pixel rounding.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoxModel {
    /// Measured outer border bounds, including element/scroll offsets.
    pub border: Layout,
    /// Border bounds inset by resolved border widths.
    pub padding: Option<Layout>,
    /// Padding bounds inset by resolved padding; absent for percentage padding.
    pub content: Option<Layout>,
    /// Border bounds expanded by resolved margins; absent for auto/percentage margins.
    pub margin: Option<Layout>,
    /// Why a box could not be determined through GPUI's public API.
    pub unavailable: Vec<String>,
}

/// Observed origin of a resolved value, without claiming Rust declaration locations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StyleOrigin {
    /// GPUI's element defaults.
    Default,
    /// Text supplied by the paint context, possibly through uninspected ancestors.
    Inherited,
    /// Explicit base refinement on this element.
    Authored,
    /// Current visual value differs from the default plus base refinement.
    /// GPUI does not expose which hover/focus/active/drag rule supplied it.
    Interaction,
    /// Actual measured size, rather than the requested size.
    Layout,
}

impl StyleOrigin {
    /// Human-readable provenance suitable for inspector details.
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "GPUI defaults",
            Self::Inherited => "Inherited text context",
            Self::Authored => "Authored base",
            Self::Interaction => "Active interaction change",
            Self::Layout => "Measured layout",
        }
    }
}

/// A disjoint group of resolved properties sharing an observed origin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StyleLayer {
    /// Observed source category; not a browser CSS cascade.
    pub origin: StyleOrigin,
    /// Resolved values in the same GPUI shape as [`ResolvedStyle::properties`].
    pub properties: Value,
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

/// An authored style attribute, without evaluating its expression or exposing text content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StyleSource {
    /// Authored tag, including an inline paragraph tag when applicable.
    pub scope: String,
    /// `class`, `class:utility`, `style`, or another style-related attribute.
    pub attribute: String,
    /// Bounded Rust expression spelling; does not indicate which conditional branch ran.
    pub expression: String,
    /// Attribute location, not an inferred location inside an opaque closure.
    pub source: SourceLocation,
}

/// Automatic `view!` boundary metadata. Inline paragraph runs share their paragraph's bounds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViewMetadata {
    /// Authored tag/component name; `Text` and `Expression` identify child boundaries.
    pub tag: String,
    /// Authored style attributes in source order, including paragraph inline attributes.
    pub style_sources: Vec<StyleSource>,
}

/// GPUI's current visual style sampled during paint, including inherited text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResolvedStyle {
    /// Complete resolved GPUI style; relative layout lengths retain their units.
    pub properties: Value,
    /// Whether GPUI's actual hitbox is hovered in this frame.
    pub hovered: bool,
    /// Property provenance; absent on older runtimes. Equal-valued interaction
    /// declarations cannot be distinguished from the base through GPUI's public API.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<StyleLayer>,
}

/// Development navigation supported by one presented inspection boundary.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Navigation {
    /// The boundary registered its real GPUI focus handle.
    pub focus: bool,
    /// Whether that registered handle currently owns keyboard focus.
    #[serde(default)]
    pub focused: bool,
    /// Registered scroll ancestors can reveal it, or it is already fully visible.
    pub scroll_into_view: bool,
}

/// One explicit navigation action; never an application command or arbitrary evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NavigationAction {
    /// Focus the registered target and activate its native window.
    Focus,
    /// Reveal the boundary through its registered scroll ancestors.
    ScrollIntoView,
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
    /// Explicit supported navigation; older runtimes report neither action.
    #[serde(default)]
    pub navigation: Navigation,
    /// Nearest retained inspected ancestor; uninstrumented intermediates are omitted.
    pub parent: Option<String>,
    /// Rust element type, without a promise of a stable wire spelling.
    pub kind: String,
    /// Application-supplied identity, distinct from GPUI state identity.
    pub semantic_id: Option<String>,
    /// Construction site of the boundary, when supplied.
    pub source: Option<SourceLocation>,
    /// Macro-authored source metadata; absent for manual boundaries and older runtimes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewMetadata>,
    /// Actual GPUI layout, or None for invalid/non-finite geometry.
    pub layout: Option<Layout>,
    /// Resolved box geometry for supported elements; older runtimes omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub box_model: Option<BoxModel>,
    /// Visible content-mask intersection in window-local logical pixels.
    #[serde(default)]
    pub content_mask: Option<Layout>,
    /// Serialized GPUI base style refinement; None for custom unstyled elements.
    pub styles: Option<Value>,
    /// Live resolved style for GPUI `Div`/`Stateful<Div>` and macro-authored buttons; absent for opaque/custom elements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<ResolvedStyle>,
    /// Whether a metadata field exceeded its capture limit.
    pub metadata_truncated: bool,
    /// Presented route owner; None for the workspace shell.
    pub tab_id: Option<TabId>,
    /// Presented route entry, never a hidden retained history entry.
    pub entry_id: Option<EntryId>,
}

impl Element {
    /// Authored view tag when known, otherwise a shortened Rust element name.
    pub fn display_name(&self) -> &str {
        self.view
            .as_ref()
            .map_or_else(|| display_name(&self.kind), |view| &view.tag)
    }
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
    /// The boundary has no registered handle for the requested action.
    #[error("navigation is unavailable for this boundary; register its focus/scroll handles")]
    NavigationUnavailable,
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
