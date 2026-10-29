//! Bounded, owned inspection of the presented view tree.
//!
//! GPUI's public API does not enumerate arbitrary descendants. Rake captures its
//! shell and application-supplied inspection boundaries, preserving their nearest
//! inspected ancestor. Layout is measured by GPUI. Authored refinements and live
//! resolved styles are separate; opaque custom elements may not expose resolution.
//! No views, text contents or source contents are retained.

#[cfg(feature = "gpui")]
mod view;
#[cfg(feature = "experimental-view")]
pub(crate) use view::authored;

#[cfg(feature = "gpui")]
pub use view::{InspectElement, Inspected, inspect};

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;
use web_time::Instant;

use crate::router::WindowId;

pub use rake_protocol::inspection::*;

#[derive(Clone)]
pub(crate) struct Store(Rc<RefCell<State>>);

struct State {
    latest: Rc<Snapshot>,
    #[cfg(any(feature = "gpui", test))]
    next_capture: u64,
    #[cfg(feature = "gpui")]
    next_boundary: u64,
    windows: BTreeMap<WindowId, WindowState>,
    mode_until: Option<Instant>,
    pick: Pick,
    pick_window: Option<WindowId>,
    #[cfg(any(feature = "gpui", test))]
    suppress_up: bool,
    changed: event_listener::Event,
}

struct WindowState {
    snapshot: Rc<Snapshot>,
    #[cfg(feature = "gpui")]
    identities: Vec<view::BoundaryIdentity>,
    #[cfg(feature = "gpui")]
    navigation: BTreeMap<String, view::NavigationTarget>,
    #[cfg(feature = "gpui")]
    pending_focus: Option<(String, gpui::FocusHandle)>,
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
            latest: Rc::new(Snapshot {
                status: Status::Unavailable,
                capture: "0".into(),
                router_revision: 0,
                elements: Vec::new(),
                omitted: 0,
                coverage: "Rake inspection boundaries, including automatic view! nodes; opaque GPUI descendants and reused cached subtrees are not enumerated. Layout is before clipping; content_mask records the visible intersection. Authored styles are separate from live resolved Div/Stateful<Div>/view! button styles, which include current interaction states and inherited text. Relative layout lengths retain their units. Box geometry resolves absolute edges from the layout-phase style; used auto/percentage edges and scrollbar metrics may be unavailable. Style origins describe observed defaults, inherited text, authored base, interaction changes and measured size; macro-authored style attribute locations are captured separately, while opaque per-method declaration locations and equal-valued overrides are unavailable. Text content is not captured.".into(),
            }),
            #[cfg(any(feature = "gpui",test))]
            next_capture: 0,
            #[cfg(feature = "gpui")]
            next_boundary: 0,
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
    pub(crate) fn navigate(
        &self,
        id: &str,
        generation: &str,
        action: NavigationAction,
    ) -> Result<(), InspectionError> {
        self.available()?;
        let mut state = self.0.borrow_mut();
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
            .ok_or(InspectionError::NotRetained)?;
        let supported = match action {
            NavigationAction::Focus => element.navigation.focus,
            NavigationAction::ScrollIntoView => element.navigation.scroll_into_view,
        };
        if !supported {
            return Err(InspectionError::NavigationUnavailable);
        }

        #[cfg(feature = "gpui")]
        {
            let target = host
                .navigation
                .get(id)
                .ok_or(InspectionError::NavigationUnavailable)?;
            match action {
                NavigationAction::Focus => {
                    host.pending_focus = Some((
                        id.into(),
                        target
                            .focus
                            .clone()
                            .ok_or(InspectionError::NavigationUnavailable)?,
                    ));
                }
                NavigationAction::ScrollIntoView => {
                    target.reveal(
                        element
                            .layout
                            .ok_or(InspectionError::NavigationUnavailable)?,
                    );
                }
            }
            state.changed.notify(usize::MAX);
            Ok(())
        }
        #[cfg(not(feature = "gpui"))]
        Err(InspectionError::NavigationUnavailable)
    }

    #[cfg(feature = "gpui")]
    fn take_focus(&self, window: WindowId) -> Option<gpui::FocusHandle> {
        let mut state = self.0.borrow_mut();
        let host = state.windows.get_mut(&window)?;
        let (id, focus) = host.pending_focus.take()?;
        host.snapshot
            .elements
            .iter()
            .any(|element| element.id == id)
            .then_some(focus)
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        let state = self.0.borrow();

        state
            .pick_window
            .and_then(|id| state.windows.get(&id))
            .map(|host| &host.snapshot)
            .unwrap_or(&state.latest)
            .as_ref()
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
        Rc::make_mut(&mut self.0.borrow_mut().latest).status = Status::Waiting;
    }

    #[cfg(any(feature = "gpui", test))]
    pub(crate) fn publish(&self, window: WindowId, mut snapshot: Snapshot) {
        let mut state = self.0.borrow_mut();

        if let Some(host) = state.windows.get(&window) {
            // Style-only redraws keep stable identities without copying style payloads.
            let old = &host.snapshot;

            let same_boundaries = snapshot.elements.len() == old.elements.len()
                && snapshot.omitted == old.omitted
                && snapshot.elements.iter().zip(&old.elements).enumerate().all(
                    |(index, (element, old))| {
                        old.id
                            .strip_prefix("element-")
                            .and_then(|id| id.rsplit_once('-'))
                            == Some((host.snapshot.capture.as_str(), index.to_string().as_str()))
                            && same_parent(
                                element.parent.as_deref(),
                                old.parent.as_deref(),
                                &host.snapshot.capture,
                            )
                            && element.kind == old.kind
                            && element.stable_id == old.stable_id
                            && element.navigation.focus == old.navigation.focus
                            && element.navigation.scroll_into_view
                                == old.navigation.scroll_into_view
                            && element.semantic_id == old.semantic_id
                            && element.source == old.source
                            && element.view == old.view
                            && element.layout == old.layout
                            && element.box_model == old.box_model
                            && element.content_mask == old.content_mask
                            && element.tab_id == old.tab_id
                            && element.entry_id == old.entry_id
                    },
                );

            if same_boundaries {
                snapshot.capture.clone_from(&old.capture);

                for (element, old) in snapshot.elements.iter_mut().zip(&old.elements) {
                    element.id.clone_from(&old.id);
                    element.parent.clone_from(&old.parent);
                }
            }
        }

        let snapshot = Rc::new(snapshot);

        let host = state.windows.entry(window).or_insert_with(|| WindowState {
            snapshot: Rc::clone(&snapshot),
            #[cfg(feature = "gpui")]
            identities: Vec::new(),
            #[cfg(feature = "gpui")]
            navigation: BTreeMap::new(),
            #[cfg(feature = "gpui")]
            pending_focus: None,
            highlight: None,
            pointer: None,
            #[cfg(feature = "gpui")]
            timer: None,
            #[cfg(feature = "gpui")]
            escape: None,
        });

        if host.snapshot.capture != snapshot.capture {
            host.highlight = host.highlight.as_ref().and_then(|(highlight, until)| {
                highlight.stable_id.as_ref().and_then(|stable| {
                    snapshot
                        .elements
                        .iter()
                        .find(|element| element.stable_id.as_ref() == Some(stable))
                        .map(|element| (element.clone(), *until))
                })
            });
        }

        host.snapshot = Rc::clone(&snapshot);
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
            .is_some_and(|(bounds, mask)| contains(bounds, x, y) && contains(mask, x, y))
    })
}

#[cfg(any(feature = "gpui", test))]
fn contains(bounds: Layout, x: f32, y: f32) -> bool {
    x >= bounds.x
        && y >= bounds.y
        && x < bounds.x + bounds.width
        && y < bounds.y + bounds.height
        && bounds.width > 0.
        && bounds.height > 0.
}

#[cfg(feature = "gpui")]
fn bound_resolved_styles(snapshot: &mut Snapshot) {
    let mut base_bytes = 0;

    for element in &mut snapshot.elements {
        let resolved = element.resolved.take();

        base_bytes += serde_json::to_vec(&*element).map_or(MAX_SNAPSHOT_BYTES, |bytes| bytes.len());
        element.resolved = resolved;
    }

    let mut remaining = (MAX_SNAPSHOT_BYTES - 4096).saturating_sub(base_bytes);

    for element in &mut snapshot.elements {
        if let Some(resolved) = &element.resolved {
            let bytes =
                serde_json::to_vec(resolved).map_or(MAX_SNAPSHOT_BYTES, |bytes| bytes.len()) + 16;

            if bytes > remaining || bytes > MAX_RESOLVED_STYLE_BYTES {
                element.resolved = None;
                element.metadata_truncated = true;
            } else {
                remaining -= bytes;
            }
        }
    }
}

// Leaves room for envelope, coverage and JSON separators below the wire limit.
#[cfg(any(feature = "gpui", test))]
fn retain(snapshot: &mut Snapshot, element: Element, bytes: &mut usize) -> Option<String> {
    // Presentation identities are assigned after traversal; reserve their maximum wire size.
    let size = serde_json::to_vec(&element).map_or(MAX_SNAPSHOT_BYTES, |value| value.len()) + 64;

    if snapshot.elements.len() >= MAX_ELEMENTS || *bytes + size >= MAX_SNAPSHOT_BYTES - 4096 {
        snapshot.omitted += 1;

        return None;
    }

    *bytes += size;
    let id = element.id.clone();

    snapshot.elements.push(element);

    Some(id)
}

#[cfg(test)]
mod tests;

#[cfg(any(feature = "gpui", test))]
fn same_parent(parent: Option<&str>, old: Option<&str>, capture: &str) -> bool {
    match parent.and_then(|parent| parent.rsplit_once('-')) {
        Some((_, index)) => {
            old.and_then(|old| old.strip_prefix("element-"))
                .and_then(|old| old.rsplit_once('-'))
                == Some((capture, index))
        }
        None => parent == old,
    }
}
