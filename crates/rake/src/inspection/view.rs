//! GPUI adapter: transparent delegation preserves layout, IDs and accessibility.

// shortcut: opaque/cached GPUI descendants need a future public full-tree capture hook.

use std::panic::Location;

use gpui::{
    App, Bounds, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId, Pixels,
    Styled, Window,
};

use super::{Element, Layout, Snapshot, SourceLocation, Status, Store};
use crate::router::{EntryId, TabId};

/// Adds a named inspection boundary to a styled GPUI element. Call after styling.
pub trait InspectElement: IntoElement + Styled {
    /// Captures authored styles, construction site and an optional semantic identity.
    /// This does not change the element's GPUI ID or make hidden history visible.
    #[track_caller]
    fn inspect(mut self, semantic_id: impl Into<String>) -> Inspected<Self::Element> {
        let style = self.style().clone();
        let mut element = inspect(self);

        element.semantic_id = Some(semantic_id.into());
        element.style = Some(style);

        element
    }
}

impl<T: IntoElement + Styled> InspectElement for T {}

/// Wraps an arbitrary element, including custom elements without styled refinements.
/// Descendants need their own boundaries to appear in the captured tree.
#[track_caller]
pub fn inspect<E: IntoElement>(element: E) -> Inspected<E::Element> {
    Inspected {
        inner: element.into_element(),
        semantic_id: None,
        source: Location::caller(),
        style: None,
        root: None,
        owner: None,
    }
}

/// Transparent GPUI element with explicit inspection metadata.
pub struct Inspected<E: gpui::Element> {
    inner: E,
    semantic_id: Option<String>,
    source: &'static Location<'static>,
    style: Option<gpui::StyleRefinement>,
    root: Option<(Store, u64)>,
    owner: Option<(TabId, EntryId)>,
}

impl<E: gpui::Element> Inspected<E> {
    /// Supplies a descriptive identity. Duplicates are allowed; lookup uses capture IDs.
    pub fn semantic_id(mut self, id: impl Into<String>) -> Self {
        self.semantic_id = Some(id.into());

        self
    }

    pub(crate) fn root(mut self, store: Store, revision: u64) -> Self {
        if store.status() != Status::Unavailable {
            self.root = Some((store, revision));
        }

        self
    }

    pub(crate) fn owner(mut self, tab: TabId, entry: EntryId) -> Self {
        self.owner = Some((tab, entry));

        self
    }
}

struct Capture {
    snapshot: Snapshot,
    parents: Vec<Option<String>>,
    owners: Vec<Option<(TabId, EntryId)>>,
    bytes: usize,
}

#[derive(Default)]
struct ActiveCapture(Option<Capture>);

impl gpui::Global for ActiveCapture {}

impl<E: gpui::Element> IntoElement for Inspected<E> {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl<E: gpui::Element> gpui::Element for Inspected<E> {
    type RequestLayoutState = E::RequestLayoutState;
    type PrepaintState = E::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        self.inner.id()
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        self.inner.source_location()
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.inner.request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        if cx.try_global::<ActiveCapture>().is_none() {
            cx.set_global(ActiveCapture::default());
        }

        let previous = self.root.as_ref().map(|(store, revision)| {
            let snapshot = store.begin(*revision);

            cx.global_mut::<ActiveCapture>().0.replace(Capture {
                snapshot,
                parents: Vec::new(),
                owners: Vec::new(),
                bytes: 0,
            })
        });

        if let Some(capture) = cx.global_mut::<ActiveCapture>().0.as_mut() {
            let owner = self
                .owner
                .or_else(|| capture.owners.last().copied().flatten());

            let mut truncated = false;

            let semantic_id = self
                .semantic_id
                .as_ref()
                .map(|value| limited(value, 256, &mut truncated));

            let kind = limited(std::any::type_name::<E>(), 256, &mut truncated);
            let file = limited(self.source.file(), 512, &mut truncated);

            let styles = self.style.as_ref().and_then(|style| {
                let value = serde_json::to_value(style).ok()?;

                if serde_json::to_vec(&value).ok()?.len() > 4096 {
                    truncated = true;

                    None
                } else {
                    Some(value)
                }
            });

            let layout = Layout {
                x: bounds.origin.x.into(),
                y: bounds.origin.y.into(),
                width: bounds.size.width.into(),
                height: bounds.size.height.into(),
            };

            let valid = [layout.x, layout.y, layout.width, layout.height]
                .iter()
                .all(|value| value.is_finite());

            let element = Element {
                id: format!(
                    "element-{}-{}",
                    capture.snapshot.capture,
                    capture.snapshot.elements.len()
                ),
                parent: capture.parents.last().cloned().flatten(),
                kind,
                semantic_id,
                source: Some(SourceLocation {
                    file,
                    line: self.source.line(),
                    column: self.source.column(),
                }),
                layout: valid.then_some(layout),
                styles,
                metadata_truncated: truncated,
                tab_id: owner.map(|owner| owner.0),
                entry_id: owner.map(|owner| owner.1),
            };

            let parent = capture
                .snapshot
                .retain(element, &mut capture.bytes)
                .or_else(|| capture.parents.last().cloned().flatten());

            capture.parents.push(parent);
            capture.owners.push(owner);
        }

        let result = self
            .inner
            .prepaint(id, inspector_id, bounds, state, window, cx);

        if let Some(capture) = cx.global_mut::<ActiveCapture>().0.as_mut() {
            capture.parents.pop();
            capture.owners.pop();
        }

        if let Some(previous) = previous {
            let completed = std::mem::replace(&mut cx.global_mut::<ActiveCapture>().0, previous);

            if let (Some((store, _)), Some(completed)) = (&self.root, completed) {
                store.publish(completed.snapshot);
            }
        }

        result
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner
            .paint(id, inspector_id, bounds, state, prepaint, window, cx);
    }

    fn a11y_role(&self) -> Option<gpui::accesskit::Role> {
        self.inner.a11y_role()
    }

    fn write_a11y_info(&self, node: &mut gpui::accesskit::Node) {
        self.inner.write_a11y_info(node);
    }

    fn a11y_synthetic_children(
        &mut self,
        prepaint: &mut Self::PrepaintState,
        builder: &mut gpui::A11ySubtreeBuilder,
    ) {
        self.inner.a11y_synthetic_children(prepaint, builder);
    }
}

fn limited(value: &str, limit: usize, truncated: &mut bool) -> String {
    if value.len() <= limit {
        return value.into();
    }

    *truncated = true;
    let mut end = limit;

    while !value.is_char_boundary(end) {
        end -= 1;
    }

    value[..end].into()
}
