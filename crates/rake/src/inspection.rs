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
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;
use web_time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::router::{EntryId, TabId, WindowId};

/// Maximum nodes in a captured tree. Remaining nodes are counted as omitted.
pub const MAX_ELEMENTS: usize = 256;
/// Maximum serialized snapshot size, below the transport's 1 MiB limit.
pub const MAX_SNAPSHOT_BYTES: usize = 512 * 1024;

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

/// One boundary in prepaint traversal order. Semantic IDs may repeat in a tree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    /// Visible content-mask intersection in window-local logical pixels.
    #[serde(default)]
    pub content_mask: Option<Layout>,
    /// Serialized GPUI base style refinement; None for custom unstyled elements.
    pub styles: Option<Value>,
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
/// assert_eq!(rake::inspection::display_name("gpui::Stateful<gpui::Div>"), "Div");
/// assert_eq!(rake::inspection::display_name("gpui::ViewElement<app::TabBar<app::Route>>"), "TabBar");
/// ```
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

#[derive(Clone)]
pub(crate) struct Store(Rc<RefCell<State>>);

struct State {
    latest: Snapshot,
    #[cfg(any(feature = "gpui", test))]
    next_capture: u64,
    windows: BTreeMap<WindowId, WindowState>,
    mode_until: Option<Instant>,
    pick: Pick,
    pick_window: Option<WindowId>,
    #[cfg(any(feature = "gpui", test))]
    suppress_up: bool,
    changed: event_listener::Event,
}

struct WindowState {
    snapshot: Snapshot,
    highlight: Option<(Element, Instant)>,
    #[cfg(any(feature = "gpui", test))]
    pointer: Option<(f32, f32)>,
    #[cfg(feature = "gpui")]
    timer: Option<(Instant, gpui::Task<()>)>,
    #[cfg(feature = "gpui")]
    escape: Option<gpui::Subscription>,
}

impl Default for Store {
    fn default() -> Self {
        Self(Rc::new(RefCell::new(State {
            latest: Snapshot {
                status: Status::Unavailable,
                capture: "0".into(),
                router_revision: 0,
                elements: Vec::new(),
                omitted: 0,
                coverage: "Rake inspection boundaries only; opaque GPUI descendants and reused cached subtrees are not enumerated. Layout is before clipping; content_mask records the visible intersection. Styles are authored base refinements, not computed, inherited or interaction styles. Text content is not captured.".into(),
            },
            #[cfg(any(feature = "gpui",test))]
            next_capture: 0,
            windows: BTreeMap::new(),
            mode_until: None,
            pick: Pick::default(),
            pick_window: None,
            #[cfg(any(feature = "gpui",test))]
            suppress_up: false,
            changed: event_listener::Event::new(),
        })))
    }
}

impl Store {
    pub(crate) fn snapshot(&self) -> Snapshot {
        let state = self.0.borrow();
        state
            .pick_window
            .and_then(|id| state.windows.get(&id))
            .map(|host| &host.snapshot)
            .unwrap_or(&state.latest)
            .clone()
    }

    pub(crate) fn element(&self, id: &str) -> Result<Element, InspectionError> {
        let state = self.0.borrow();
        match state.latest.status {
            Status::Unavailable => Err(InspectionError::Unavailable),
            Status::Waiting => Err(InspectionError::Waiting),
            Status::Captured => state
                .windows
                .values()
                .flat_map(|host| &host.snapshot.elements)
                .chain(&state.latest.elements)
                .find(|element| element.id == id)
                .cloned()
                .ok_or(InspectionError::NotRetained),
        }
    }

    pub(crate) fn status(&self) -> Status {
        self.0.borrow().latest.status
    }

    pub(crate) fn highlight(
        &self,
        id: Option<&str>,
        generation: &str,
    ) -> Result<(), InspectionError> {
        self.available()?;
        let mut state = self.0.borrow_mut();
        if let Some(id) = id {
            let host = state
                .windows
                .values_mut()
                .find(|host| host.snapshot.capture == generation)
                .ok_or(InspectionError::StaleGeneration)?;
            let element = host
                .snapshot
                .elements
                .iter()
                .find(|element| element.id == id)
                .ok_or(InspectionError::NotRetained)?
                .clone();
            host.highlight = Some((element, Instant::now() + Duration::from_secs(2)));
        } else {
            for host in state.windows.values_mut() {
                host.highlight = None;
            }
        }
        state.changed.notify(usize::MAX);
        Ok(())
    }

    #[cfg(feature = "gpui")]
    pub(crate) fn listen(&self) -> event_listener::EventListener {
        self.0.borrow().changed.listen()
    }

    fn available(&self) -> Result<(), InspectionError> {
        match self.status() {
            Status::Unavailable => Err(InspectionError::Unavailable),
            Status::Waiting => Err(InspectionError::Waiting),
            Status::Captured => Ok(()),
        }
    }

    pub(crate) fn inspect_mode(&self, enabled: bool, lease_ms: u64) -> Result<(), InspectionError> {
        self.available()?;
        if enabled && !(1..=2000).contains(&lease_ms) {
            return Err(InspectionError::InvalidLease);
        }
        let mut state = self.0.borrow_mut();
        if enabled {
            if !state.mode_active(Instant::now()) {
                state.pick.hovered = None;
                state.pick.picked = None;
                state.pick_window = None;
            }
            state.mode_until = Some(Instant::now() + Duration::from_millis(lease_ms));
            for host in state.windows.values_mut() {
                if let Some((_, until)) = &mut host.highlight {
                    *until = Instant::now() + Duration::from_secs(2);
                }
            }
        } else {
            state.cancel();
        }
        state.changed.notify(usize::MAX);
        Ok(())
    }

    pub(crate) fn pick(&self) -> Result<Pick, InspectionError> {
        self.available()?;
        let mut state = self.0.borrow_mut();
        state.expire(Instant::now());
        let mut pick = state.pick.clone();
        pick.mode = state.mode_active(Instant::now());
        Ok(pick)
    }

    #[cfg(any(feature = "gpui", test))]
    fn pointer(&self, window: WindowId, x: f32, y: f32, click: bool) -> bool {
        let mut state = self.0.borrow_mut();
        let now = Instant::now();
        state.expire(now);
        if !state.mode_active(now) {
            return false;
        }
        state.hover(window, x, y, now);
        if click {
            state.pick.picked = state.pick.hovered.clone();
            state.pick.pick_sequence = state.pick.pick_sequence.saturating_add(1);
            state.mode_until = None;
            state.suppress_up = true;
        }
        true
    }

    #[cfg(feature = "gpui")]
    pub(crate) fn detach_window(&self, window: WindowId) {
        let mut state = self.0.borrow_mut();
        state.windows.remove(&window);
        if state.pick_window == Some(window) {
            state.pick_window = None;
            state.cancel();
        }
    }

    #[cfg(any(feature = "gpui", test))]
    pub(crate) fn begin(&self, revision: u64) -> Snapshot {
        let mut state = self.0.borrow_mut();
        state.next_capture = state
            .next_capture
            .checked_add(1)
            .expect("inspection capture overflow");
        Snapshot {
            capture: state.next_capture.to_string(),
            status: Status::Captured,
            router_revision: revision,
            elements: Vec::new(),
            omitted: 0,
            coverage: state.latest.coverage.clone(),
        }
    }

    #[cfg(feature = "gpui")]
    pub(crate) fn attach(&self) {
        self.0.borrow_mut().latest.status = Status::Waiting;
    }

    #[cfg(any(feature = "gpui", test))]
    pub(crate) fn publish(&self, window: WindowId, mut snapshot: Snapshot) {
        let mut state = self.0.borrow_mut();
        if let Some(host) = state.windows.get(&window) {
            // Overlay-only redraws must not invalidate the node the user is picking.
            let mut stable = snapshot.clone();
            stable.capture = host.snapshot.capture.clone();
            for (index, element) in stable.elements.iter_mut().enumerate() {
                element.id = format!("element-{}-{index}", stable.capture);
                if let Some(parent) = &element.parent
                    && let Some((_, index)) = parent.rsplit_once('-')
                {
                    element.parent = Some(format!("element-{}-{index}", stable.capture));
                }
            }
            if stable.elements == host.snapshot.elements && stable.omitted == host.snapshot.omitted
            {
                snapshot = stable;
            }
        }
        let host = state.windows.entry(window).or_insert_with(|| WindowState {
            snapshot: snapshot.clone(),
            highlight: None,
            pointer: None,
            #[cfg(feature = "gpui")]
            timer: None,
            #[cfg(feature = "gpui")]
            escape: None,
        });
        if host.snapshot.capture != snapshot.capture {
            host.highlight = None;
        }
        host.snapshot = snapshot.clone();
        let pointer = host.pointer;
        state.latest = snapshot;
        if state.mode_active(Instant::now())
            && state.pick_window == Some(window)
            && let Some((x, y)) = pointer
        {
            state.hover(window, x, y, Instant::now());
        }
    }
}

impl State {
    fn mode_active(&self, now: Instant) -> bool {
        self.mode_until.is_some_and(|until| until > now)
    }

    fn cancel(&mut self) {
        self.mode_until = None;
        self.pick.hovered = None;
        for host in self.windows.values_mut() {
            host.highlight = None;
        }
    }

    fn expire(&mut self, now: Instant) {
        if self.mode_until.is_some_and(|until| until <= now) {
            self.cancel();
        }
        for host in self.windows.values_mut() {
            if host
                .highlight
                .as_ref()
                .is_some_and(|(_, until)| *until <= now)
            {
                host.highlight = None;
            }
        }
    }

    #[cfg(any(feature = "gpui", test))]
    fn hover(&mut self, window: WindowId, x: f32, y: f32, now: Instant) {
        if let Some(previous) = self.pick_window.filter(|previous| *previous != window)
            && let Some(host) = self.windows.get_mut(&previous)
        {
            host.highlight = None;
        }
        let Some(host) = self.windows.get_mut(&window) else {
            return;
        };
        host.pointer = Some((x, y));
        let element = hit_test(&host.snapshot, x, y).cloned();
        let id = element.as_ref().map(|e| e.id.clone());
        if self.pick.hovered != id || self.pick.generation != host.snapshot.capture {
            host.highlight = element.map(|e| (e, now + Duration::from_secs(2)));
        }
        self.pick.generation = host.snapshot.capture.clone();
        self.pick.hovered = id;
        self.pick_window = Some(window);
    }
}

#[cfg(any(feature = "gpui", test))]
fn hit_test(snapshot: &Snapshot, x: f32, y: f32) -> Option<&Element> {
    snapshot.elements.iter().rev().find(|element| {
        element
            .layout
            .zip(element.content_mask)
            .is_some_and(|(bounds, mask)| bounds.contains(x, y) && mask.contains(x, y))
    })
}

impl Layout {
    #[cfg(any(feature = "gpui", test))]
    fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x
            && y >= self.y
            && x < self.x + self.width
            && y < self.y + self.height
            && self.width > 0.
            && self.height > 0.
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
