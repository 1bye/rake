//! GPUI adapter: transparent delegation preserves layout, IDs and accessibility.

// shortcut: opaque/cached GPUI descendants need a future public full-tree capture hook.

use std::panic::Location;

use gpui::{
    App, Bounds, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId, Pixels,
    Styled, Window,
};

use super::{Element, Layout, Snapshot, SourceLocation, Status, Store};
use crate::router::{EntryId, TabId, WindowId};

#[cfg(feature = "experimental-view")]
pub(crate) mod authored;
mod geometry;
mod provenance;

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
        focus: None,
        scroll: None,
        #[cfg(feature = "experimental-view")]
        definition: None,
        #[cfg(feature = "experimental-view")]
        view_key: None,
        style_reader: None,
        original_style: None,
    }
}

/// Transparent GPUI element with explicit inspection metadata.
pub struct Inspected<E: gpui::Element> {
    inner: E,
    semantic_id: Option<String>,
    source: &'static Location<'static>,
    style: Option<gpui::StyleRefinement>,
    root: Option<(Store, u64, WindowId)>,
    owner: Option<(TabId, EntryId)>,
    focus: Option<gpui::FocusHandle>,
    scroll: Option<gpui::ScrollHandle>,
    #[cfg(feature = "experimental-view")]
    definition: Option<&'static authored::Definition>,
    #[cfg(feature = "experimental-view")]
    view_key: Option<ElementId>,
    style_reader: Option<fn(&mut E) -> gpui::StyleRefinement>,
    original_style: Option<gpui::StyleRefinement>,
}

impl<E: gpui::Element + Styled> Styled for Inspected<E> {
    fn style(&mut self) -> &mut gpui::StyleRefinement {
        self.style_reader = Some(|inner| inner.style().clone());
        self.inner.style()
    }
}

impl<E: gpui::Element + gpui::ParentElement> gpui::ParentElement for Inspected<E> {
    fn extend(&mut self, elements: impl IntoIterator<Item = gpui::AnyElement>) {
        self.inner.extend(elements);
    }
}

impl<E: gpui::Element + gpui::InteractiveElement> gpui::InteractiveElement for Inspected<E> {
    fn interactivity(&mut self) -> &mut gpui::Interactivity {
        self.inner.interactivity()
    }
}

impl<E: gpui::Element + gpui::StatefulInteractiveElement> gpui::StatefulInteractiveElement
    for Inspected<E>
{
}

impl<E: gpui::Element + gpui::StyledImage> gpui::StyledImage for Inspected<E> {
    fn image_style(&mut self) -> &mut gpui::ImageStyle {
        self.inner.image_style()
    }
}

impl<E: gpui::Element> Inspected<E> {
    fn boundary_key(
        &self,
        id: Option<&GlobalElementId>,
        owner: Option<(TabId, EntryId)>,
    ) -> BoundaryKey {
        BoundaryKey {
            gpui_id: id.cloned(),
            kind: std::any::type_name::<E>(),
            source: self.source,
            owner,
            #[cfg(feature = "experimental-view")]
            view_site: self.definition.map(|definition| definition.site),
            #[cfg(feature = "experimental-view")]
            view_key: self.view_key.clone(),
        }
    }

    /// Extracts the underlying GPUI element, removing this boundary only.
    pub fn into_inner(self) -> E {
        self.inner
    }

    /// Renames this boundary without inserting a duplicate inspection wrapper.
    pub fn inspect(mut self, semantic_id: impl Into<String>) -> Self {
        self.semantic_id = Some(semantic_id.into());
        self
    }

    /// Registers the same handle passed to GPUI's `track_focus`, without changing focusability.
    pub fn focus_handle(mut self, handle: &gpui::FocusHandle) -> Self {
        self.focus = Some(handle.clone());
        self
    }

    /// Registers the same handle passed to `track_scroll` so inspected descendants can be revealed.
    pub fn scroll_handle(mut self, handle: &gpui::ScrollHandle) -> Self {
        self.scroll = Some(handle.clone());
        self
    }

    /// Supplies a descriptive identity. Duplicates are allowed; lookup uses capture IDs.
    pub fn semantic_id(mut self, id: impl Into<String>) -> Self {
        self.semantic_id = Some(id.into());

        self
    }

    pub(crate) fn root(mut self, store: Store, revision: u64, window: WindowId) -> Self {
        if store.status() != Status::Unavailable {
            self.root = Some((store, revision, window));
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
    next_paint: usize,
    candidates: Vec<BoundaryKey>,
    scrolls: Vec<gpui::ScrollHandle>,
    navigation: Vec<NavigationTarget>,
}

#[derive(Clone, Default)]
pub(super) struct NavigationTarget {
    pub focus: Option<gpui::FocusHandle>,
    scrolls: Vec<gpui::ScrollHandle>,
}

impl NavigationTarget {
    pub fn reveal(&self, layout: Layout) {
        let mut bounds = Bounds {
            origin: gpui::point(gpui::px(layout.x), gpui::px(layout.y)),
            size: gpui::size(gpui::px(layout.width), gpui::px(layout.height)),
        };
        for scroll in self.scrolls.iter().rev() {
            let viewport = scroll.bounds();
            let delta = gpui::point(
                reveal_delta(
                    bounds.left(),
                    bounds.right(),
                    viewport.left(),
                    viewport.right(),
                ),
                reveal_delta(
                    bounds.top(),
                    bounds.bottom(),
                    viewport.top(),
                    viewport.bottom(),
                ),
            );
            let old = scroll.offset();
            let max = scroll.max_offset();
            let offset = gpui::point(
                (old.x - delta.x).clamp(-max.x, gpui::px(0.)),
                (old.y - delta.y).clamp(-max.y, gpui::px(0.)),
            );
            scroll.set_offset(offset);
            bounds.origin += offset - old;
        }
    }
}

fn reveal_delta(
    start: Pixels,
    end: Pixels,
    viewport_start: Pixels,
    viewport_end: Pixels,
) -> Pixels {
    if start < viewport_start || end - start > viewport_end - viewport_start {
        start - viewport_start
    } else {
        (end - viewport_end).max(gpui::px(0.))
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct BoundaryKey {
    gpui_id: Option<GlobalElementId>,
    kind: &'static str,
    source: &'static Location<'static>,
    owner: Option<(TabId, EntryId)>,
    #[cfg(feature = "experimental-view")]
    view_site: Option<&'static str>,
    #[cfg(feature = "experimental-view")]
    view_key: Option<ElementId>,
}

#[derive(Clone)]
pub(super) struct BoundaryIdentity {
    key: BoundaryKey,
    parent: Option<String>,
    id: String,
}

impl Capture {
    fn identify(&mut self, store: &Store, window: WindowId) -> Vec<BoundaryIdentity> {
        let mut state = store.0.borrow_mut();
        let old = state
            .windows
            .get(&window)
            .map(|host| host.identities.as_slice())
            .unwrap_or_default();
        let mut next = state.next_boundary;
        let mut identities = Vec::new();

        let old: std::collections::HashMap<_, _> = old
            .iter()
            .map(|old| ((&old.key, old.parent.as_deref()), old.id.clone()))
            .collect();
        let indices: std::collections::HashMap<_, _> = self
            .snapshot
            .elements
            .iter()
            .enumerate()
            .map(|(i, e)| (e.id.clone(), i))
            .collect();
        let mut counts = std::collections::HashMap::new();
        for (key, element) in self.candidates.iter().zip(&self.snapshot.elements) {
            *counts
                .entry((key, element.parent.as_deref()))
                .or_insert(0usize) += 1;
        }
        let unique: Vec<_> = self
            .candidates
            .iter()
            .zip(&self.snapshot.elements)
            .map(|(key, element)| counts[&(key, element.parent.as_deref())] == 1)
            .collect();

        for (index, key) in self.candidates.iter().enumerate() {
            let element = &self.snapshot.elements[index];
            let parent = element
                .parent
                .as_ref()
                .and_then(|id| indices.get(id))
                .map(|index| &self.snapshot.elements[*index]);
            if parent.is_some_and(|parent| parent.stable_id.is_none()) || !unique[index] {
                continue;
            }
            let parent = parent.and_then(|parent| parent.stable_id.clone());
            let id = old
                .get(&(key, parent.as_deref()))
                .cloned()
                .unwrap_or_else(|| {
                    next = next
                        .checked_add(1)
                        .expect("inspection boundary identity overflow");
                    format!("boundary-{next}")
                });

            self.snapshot.elements[index].stable_id = Some(id.clone());
            identities.push(BoundaryIdentity {
                key: key.clone(),
                parent,
                id,
            });
        }

        state.next_boundary = next;

        identities
    }
}

#[derive(Default)]
struct ActiveCapture(Option<Capture>);

impl gpui::Global for ActiveCapture {}

#[derive(Default)]
struct CapturingLayout(bool);

#[derive(Clone)]
struct LayoutContext {
    store: Store,
    window: WindowId,
    parent: Option<String>,
    owner: Option<(TabId, EntryId)>,
}
#[derive(Default)]
struct LayoutHost(Option<LayoutContext>);
impl gpui::Global for LayoutHost {}

impl gpui::Global for CapturingLayout {}

impl<E: gpui::Element> IntoElement for Inspected<E> {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl<E: gpui::Element> gpui::Element for Inspected<E> {
    type RequestLayoutState = (E::RequestLayoutState, Option<gpui::Style>);
    type PrepaintState = (E::PrepaintState, Option<gpui::AnyElement>, Option<usize>);

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
        if let Some((store, _, host)) = &self.root
            && let Some(focus) = store.take_focus(*host)
        {
            window.activate_window();
            window.focus(&focus, cx);
        }
        if cx.try_global::<CapturingLayout>().is_none() {
            cx.set_global(CapturingLayout::default());
        }
        let previous = self.root.as_ref().map(|(store, _, _)| {
            std::mem::replace(
                &mut cx.global_mut::<CapturingLayout>().0,
                store.status() != Status::Unavailable,
            )
        });
        let capturing = cx.global::<CapturingLayout>().0;
        if cx.try_global::<LayoutHost>().is_none() {
            cx.set_global(LayoutHost::default());
        }
        let previous_host = cx.global::<LayoutHost>().0.clone();
        let context = self
            .root
            .as_ref()
            .map(|(store, _, host)| LayoutContext {
                store: store.clone(),
                window: *host,
                parent: None,
                owner: None,
            })
            .or_else(|| previous_host.clone());
        let mut child_context = context.clone();
        if let Some(context) = &context {
            let owner = self.owner.or(context.owner);
            let key = self.boundary_key(id, owner);
            let state = context.store.0.borrow();
            let stable = state
                .windows
                .get(&context.window)
                .and_then(|host| {
                    host.identities
                        .iter()
                        .find(|identity| identity.key == key && identity.parent == context.parent)
                })
                .map(|identity| identity.id.clone());
            if let Some(child) = &mut child_context {
                child.parent = stable.clone();
                child.owner = owner;
            }
            if let Some(interactive) = interactivity(&mut self.inner) {
                if let Some(original) = self.original_style.take() {
                    *interactive.base_style = original;
                }
                self.style = Some(*interactive.base_style.clone());
                if let Some(edit) = stable.as_ref().and_then(|stable| state.edits.get(stable)) {
                    self.original_style = Some(*interactive.base_style.clone());
                    if let Ok(refinement) = super::editing::refinement(&edit.current) {
                        gpui::Refineable::refine(interactive.base_style.as_mut(), &refinement);
                    }
                }
            }
        }
        cx.global_mut::<LayoutHost>().0 = child_context;
        let (layout, state) = self.inner.request_layout(id, inspector_id, window, cx);
        cx.global_mut::<LayoutHost>().0 = previous_host;
        if capturing
            && self.original_style.is_none()
            && let Some(read) = self.style_reader
        {
            self.style = Some(read(&mut self.inner));
        }
        // Observe after GPUI has initialized focus and cleared stale drag/active state.
        let style = capturing
            .then(|| {
                interactivity(&mut self.inner)
                    .map(|interactive| interactive.compute_style(id, None, window, cx))
            })
            .flatten();
        if let Some(previous) = previous {
            cx.global_mut::<CapturingLayout>().0 = previous;
        }
        (layout, (state, style))
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

        let previous = self.root.as_ref().map(|(store, revision, _)| {
            let snapshot = store.begin(*revision);

            cx.global_mut::<ActiveCapture>().0.replace(Capture {
                snapshot,
                parents: Vec::new(),
                owners: Vec::new(),
                bytes: 0,
                next_paint: 0,
                candidates: Vec::new(),
                scrolls: Vec::new(),
                navigation: Vec::new(),
            })
        });

        let mut retained_index = None;

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
                stable_id: None,
                navigation: super::Navigation {
                    focus: self.focus.is_some(),
                    focused: self
                        .focus
                        .as_ref()
                        .is_some_and(|focus| focus.is_focused(window)),
                    scroll_into_view: !capture.scrolls.is_empty()
                        || (valid && {
                            let visible = bounds
                                .intersect(&window.content_mask().bounds)
                                .intersect(&Bounds {
                                    origin: gpui::point(gpui::px(0.), gpui::px(0.)),
                                    size: window.viewport_size(),
                                });
                            visible == bounds
                        }),
                },
                parent: capture.parents.last().cloned().flatten(),
                kind,
                semantic_id,
                source: Some(SourceLocation {
                    file,
                    line: self.source.line(),
                    column: self.source.column(),
                }),
                view: {
                    #[cfg(feature = "experimental-view")]
                    {
                        self.definition
                            .map(|definition| definition.capture(&mut truncated))
                    }
                    #[cfg(not(feature = "experimental-view"))]
                    {
                        None
                    }
                },
                layout: valid.then_some(layout),
                box_model: valid
                    .then(|| {
                        state.1.as_ref().map(|style| {
                            geometry::capture(
                                style,
                                layout,
                                window.rem_size(),
                                window.scale_factor(),
                            )
                        })
                    })
                    .flatten(),
                content_mask: {
                    let visible =
                        bounds
                            .intersect(&window.content_mask().bounds)
                            .intersect(&Bounds {
                                origin: gpui::point(gpui::px(0.), gpui::px(0.)),
                                size: window.viewport_size(),
                            });

                    let mask = Layout {
                        x: visible.origin.x.into(),
                        y: visible.origin.y.into(),
                        width: visible.size.width.into(),
                        height: visible.size.height.into(),
                    };

                    [mask.x, mask.y, mask.width, mask.height]
                        .iter()
                        .all(|v| v.is_finite())
                        .then_some(mask)
                },
                styles,
                resolved: None,
                metadata_truncated: truncated,
                paint_order: None,
                style_editable: state.1.is_some(),
                style_overrides: Vec::new(),
                tab_id: owner.map(|owner| owner.0),
                entry_id: owner.map(|owner| owner.1),
            };

            let index = capture.snapshot.elements.len();
            let retained = retain(&mut capture.snapshot, element, &mut capture.bytes);

            retained_index = retained.as_ref().map(|_| index);
            if retained.is_some() {
                capture.candidates.push(self.boundary_key(id, owner));
                capture.navigation.push(NavigationTarget {
                    focus: self.focus.clone(),
                    scrolls: capture.scrolls.clone(),
                });
            }
            let parent = retained.or_else(|| capture.parents.last().cloned().flatten());

            capture.parents.push(parent);
            capture.owners.push(owner);
            if let Some(scroll) = &self.scroll {
                capture.scrolls.push(scroll.clone());
            }
        }

        let result = self
            .inner
            .prepaint(id, inspector_id, bounds, &mut state.0, window, cx);

        if let Some(capture) = cx.global_mut::<ActiveCapture>().0.as_mut() {
            capture.parents.pop();
            capture.owners.pop();
            if self.scroll.is_some() {
                capture.scrolls.pop();
            }
        }

        if let Some(previous) = previous {
            let completed = std::mem::replace(&mut cx.global_mut::<ActiveCapture>().0, previous);

            if let (Some((store, _, host)), Some(mut completed)) = (&self.root, completed) {
                let identities = completed.identify(store, *host);
                for element in &mut completed.snapshot.elements {
                    element.style_editable &= element.stable_id.is_some();
                    element.style_overrides = element
                        .stable_id
                        .as_ref()
                        .and_then(|id| {
                            store
                                .0
                                .borrow()
                                .edits
                                .get(id)
                                .map(|edit| edit.current.clone())
                        })
                        .unwrap_or_default();
                }
                store.publish(*host, completed.snapshot);
                let mut state = store.0.borrow_mut();
                let host = state
                    .windows
                    .get_mut(host)
                    .expect("published inspection host");
                host.identities = identities;
                host.navigation = host
                    .snapshot
                    .elements
                    .iter()
                    .zip(completed.navigation)
                    .map(|(element, target)| (element.id.clone(), target))
                    .collect();
            }
        }

        let overlay = self
            .root
            .as_ref()
            .and_then(|(store, _, host)| overlay(store, *host, window, cx));

        (result, overlay, retained_index)
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
        let previous = self.root.as_ref().and_then(|(store, _, host)| {
            let snapshot = store
                .0
                .borrow()
                .windows
                .get(host)?
                .snapshot
                .as_ref()
                .clone();

            Some(cx.global_mut::<ActiveCapture>().0.replace(Capture {
                snapshot,
                parents: Vec::new(),
                owners: Vec::new(),
                bytes: 0,
                next_paint: 0,
                candidates: Vec::new(),
                scrolls: Vec::new(),
                navigation: Vec::new(),
            }))
        });

        if let Some(index) = prepaint.2
            && cx.global::<ActiveCapture>().0.is_some()
        {
            let resolved = resolved_style(
                &mut self.inner,
                (&prepaint.0, self.original_style.as_ref()),
                id,
                bounds,
                window,
                cx,
            );

            if let Some(capture) = cx.global_mut::<ActiveCapture>().0.as_mut()
                && let Some(element) = capture.snapshot.elements.get_mut(index)
            {
                element.paint_order = Some(capture.next_paint);
                capture.next_paint += 1;
                let visible = bounds
                    .intersect(&window.content_mask().bounds)
                    .intersect(&Bounds {
                        origin: gpui::point(gpui::px(0.), gpui::px(0.)),
                        size: window.viewport_size(),
                    });
                element.content_mask = Some(Layout {
                    x: visible.origin.x.into(),
                    y: visible.origin.y.into(),
                    width: visible.size.width.into(),
                    height: visible.size.height.into(),
                });
                element.resolved = resolved;
            }
        }

        if let Some((store, _, host)) = &self.root {
            listeners(store, *host, window, cx);
        }

        self.inner.paint(
            id,
            inspector_id,
            bounds,
            &mut state.0,
            &mut prepaint.0,
            window,
            cx,
        );

        if let Some(previous) = previous {
            let completed = std::mem::replace(&mut cx.global_mut::<ActiveCapture>().0, previous);

            if let (Some((store, _, host)), Some(mut completed)) = (&self.root, completed) {
                bound_resolved_styles(&mut completed.snapshot);
                store.publish(*host, completed.snapshot);
            }
        }

        if let Some(overlay) = &mut prepaint.1 {
            let mask = self
                .root
                .as_ref()
                .and_then(|(store, _, host)| {
                    store
                        .0
                        .borrow()
                        .windows
                        .get(host)?
                        .highlight
                        .as_ref()?
                        .0
                        .content_mask
                })
                .map(|mask| gpui::ContentMask {
                    bounds: Bounds {
                        origin: gpui::point(gpui::px(mask.x), gpui::px(mask.y)),
                        size: gpui::size(gpui::px(mask.width), gpui::px(mask.height)),
                    },
                });
            window.with_content_mask(mask, |window| overlay.paint(window, cx));
        }
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
        self.inner.a11y_synthetic_children(&mut prepaint.0, builder);
    }
}

fn interactivity<E: gpui::Element>(inner: &mut E) -> Option<&mut gpui::Interactivity> {
    use gpui::InteractiveElement;
    use std::any::Any;
    let inner = inner as &mut dyn Any;
    #[cfg(feature = "experimental-view")]
    if inner.is::<rake_ui::view::Button>() {
        return Some(
            inner
                .downcast_mut::<rake_ui::view::Button>()?
                .interactivity(),
        );
    }
    if inner.is::<gpui::Div>() {
        Some(inner.downcast_mut::<gpui::Div>()?.interactivity())
    } else {
        Some(
            inner
                .downcast_mut::<gpui::Stateful<gpui::Div>>()?
                .interactivity(),
        )
    }
}

fn resolved_style<E: gpui::Element>(
    inner: &mut E,
    (prepaint, original): (&E::PrepaintState, Option<&gpui::StyleRefinement>),
    id: Option<&GlobalElementId>,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) -> Option<super::ResolvedStyle> {
    use gpui::Refineable;
    use std::any::Any;
    let hitbox = (prepaint as &dyn Any)
        .downcast_ref::<Option<gpui::Hitbox>>()?
        .as_ref();

    let authored = (inner as &mut dyn Any)
        .downcast_mut::<gpui::Div>()
        .map(|div| div.style().clone())
        .or_else(|| {
            (inner as &mut dyn Any)
                .downcast_mut::<gpui::Stateful<gpui::Div>>()
                .map(|div| div.style().clone())
        });
    #[cfg(feature = "experimental-view")]
    let authored = authored.or_else(|| {
        (inner as &mut dyn Any)
            .downcast_mut::<rake_ui::view::Button>()
            .map(|button| button.style().clone())
    });
    let authored = authored?;
    let mut base = gpui::Style::default();
    base.refine(&authored);
    let interactive = interactivity(inner)?;
    let style = interactive.compute_style(id, hitbox, window, cx);
    let mut text = window.text_style();

    text.refine(&style.text);
    let font_size = text.font_size.to_pixels(window.rem_size());

    text.line_height = text
        .line_height
        .to_pixels(font_size.into(), window.rem_size())
        .into();
    text.font_size = font_size.into();
    let mut properties = serde_json::to_value(style.subtract(&Default::default())).ok()?;

    properties["text"] = serde_json::to_value(text.subtract(&Default::default())).ok()?;

    if style.background.is_none() {
        properties["background"] = serde_json::Value::Null;
    }

    if style.border_color.is_none() {
        properties["border_color"] = serde_json::Value::Null;
    }

    properties["opacity"] = serde_json::json!(style.opacity.unwrap_or(1.));
    properties["size"] = serde_json::json!({
        "width":format!("{}px", f32::from(bounds.size.width)),
        "height":format!("{}px", f32::from(bounds.size.height)),
    });
    resolve_lengths(&mut properties, window.rem_size().into());

    let layers = provenance::capture(
        &properties,
        &serde_json::to_value(original.unwrap_or(&authored)).ok()?,
        &serde_json::to_value(base.subtract(&Default::default())).ok()?,
        &serde_json::to_value(style.subtract(&Default::default())).ok()?,
    );

    Some(super::ResolvedStyle {
        properties,
        hovered: hitbox.is_some_and(|hitbox| hitbox.is_hovered(window)),
        layers,
    })
}

fn resolve_lengths(properties: &mut serde_json::Value, rem_size: f32) {
    for name in [
        "size",
        "min_size",
        "max_size",
        "inset",
        "padding",
        "margin",
        "gap",
        "border_widths",
        "corner_radii",
        "flex_basis",
        "scrollbar_width",
    ] {
        if let Some(value) = properties.get_mut(name) {
            resolve_rems(value, rem_size);
        }
    }
}

fn resolve_rems(value: &mut serde_json::Value, rem_size: f32) {
    match value {
        serde_json::Value::String(text) => {
            if let Some(number) = text
                .strip_suffix("rem")
                .and_then(|text| text.parse::<f32>().ok())
            {
                *text = format!("{}px", number * rem_size);
            }
        }
        serde_json::Value::Object(fields) => {
            for value in fields.values_mut() {
                resolve_rems(value, rem_size);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                resolve_rems(value, rem_size);
            }
        }
        _ => {}
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

fn overlay(
    store: &Store,
    host: WindowId,
    window: &mut Window,
    cx: &mut App,
) -> Option<gpui::AnyElement> {
    use gpui::{div, point, prelude::*, px, rgba};
    let mut state = store.0.borrow_mut();

    state.expire(web_time::Instant::now());
    let (element, _) = state.windows.get(&host)?.highlight.as_ref()?;
    let layout = element.layout?;
    let mask = element.content_mask?;

    let label = format!(
        "{} {} · {}×{}",
        element.display_name(),
        element.semantic_id.as_deref().unwrap_or(""),
        layout.width,
        layout.height
    );

    drop(state);
    let mut element = div()
        .debug_selector(|| "rake.inspection.highlight".into())
        .w(px(layout.width))
        .h(px(layout.height))
        .bg(rgba(0x4da3ff30))
        .border_1()
        .border_color(rgba(0x4da3ffff))
        .child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .px_1()
                .bg(rgba(0x183651ed))
                .text_color(rgba(0xffffffff))
                .text_size(px(11.))
                .child(label),
        )
        .into_any_element();

    element.layout_as_root(
        window.viewport_size().map(gpui::AvailableSpace::Definite),
        window,
        cx,
    );
    // Root layout ignores positional styles; place the overlay in window coordinates.
    window.with_content_mask(
        Some(gpui::ContentMask {
            bounds: Bounds {
                origin: point(px(mask.x), px(mask.y)),
                size: gpui::size(px(mask.width), px(mask.height)),
            },
        }),
        |window| element.prepaint_at(point(px(layout.x), px(layout.y)), window, cx),
    );

    Some(element)
}

fn listeners(store: &Store, host: WindowId, window: &mut Window, cx: &mut App) {
    use gpui::{DispatchPhase, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ScrollWheelEvent};
    let moving = store.clone();

    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
        if phase == DispatchPhase::Capture
            && moving.pointer(
                host,
                event.position.x.into(),
                event.position.y.into(),
                false,
            )
        {
            cx.stop_propagation();
            window.refresh();
        }
    });
    let clicking = store.clone();

    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
        if phase == DispatchPhase::Capture
            && clicking.pointer(
                host,
                event.position.x.into(),
                event.position.y.into(),
                event.button == gpui::MouseButton::Left,
            )
        {
            cx.stop_propagation();
            window.refresh();
        }
    });
    let releasing = store.clone();

    window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
        if phase == DispatchPhase::Capture {
            let mut state = releasing.0.borrow_mut();

            state.expire(web_time::Instant::now());

            if state.mode_active(web_time::Instant::now()) || std::mem::take(&mut state.suppress_up)
            {
                cx.stop_propagation();
            }
        }
    });
    let scrolling = store.clone();

    window.on_mouse_event(move |_: &ScrollWheelEvent, phase, _, cx| {
        if phase == DispatchPhase::Capture
            && scrolling.0.borrow().mode_active(web_time::Instant::now())
        {
            cx.stop_propagation();
        }
    });

    if store
        .0
        .borrow()
        .windows
        .get(&host)
        .is_some_and(|host| host.escape.is_none())
    {
        let owner = std::rc::Rc::downgrade(&store.0);
        let id = window.window_handle().window_id();

        let subscription = cx.intercept_keystrokes(move |event, window, cx| {
            if window.window_handle().window_id() != id {
                return;
            }

            let Some(owner) = owner.upgrade() else {
                return;
            };

            let mut state = owner.borrow_mut();

            state.expire(web_time::Instant::now());

            if state.mode_active(web_time::Instant::now()) && event.keystroke.key == "escape" {
                state.cancel();
                cx.stop_propagation();
                window.refresh();
            }
        });

        if let Some(host) = store.0.borrow_mut().windows.get_mut(&host) {
            host.escape = Some(subscription);
        }
    }

    // One host-owned timer guarantees expired overlays repaint even if every client vanished.
    let now = web_time::Instant::now();
    let mut state = store.0.borrow_mut();
    let mode_until = state.mode_until;

    if let Some(host) = state.windows.get_mut(&host) {
        let deadline = host
            .highlight
            .as_ref()
            .map(|(_, until)| *until)
            .into_iter()
            .chain(mode_until)
            .min();

        if let Some(deadline) = deadline.filter(|deadline| {
            host.timer
                .as_ref()
                .is_none_or(|(until, _)| *until <= now || *deadline < *until)
        }) {
            let timer = cx
                .background_executor()
                .timer(deadline.saturating_duration_since(now));

            let task = window.spawn(cx, async move |cx| {
                timer.await;
                let _ = cx.update(|window, _| window.refresh());
            });

            host.timer = Some((deadline, task));
        }
    }
}

#[cfg(all(test, feature = "view-test-support", not(target_family = "wasm")))]
mod tests {
    //! Covers: capture-phase input suppression, click picks, Escape and expired leases.
    //! Real: GPUI window dispatch, inspection root and control store.
    //! Doubles: TestPlatform window/GPU; requires view-test-support, no display.
    use super::*;
    use gpui::{Context, InputEvent as _, Render, prelude::*};
    use std::cell::Cell;
    use std::rc::Rc;

    struct NavigationView {
        store: Store,
        focus: gpui::FocusHandle,
        scroll: gpui::ScrollHandle,
    }

    impl Render for NavigationView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            gpui::div()
                .id("scroll")
                .w(gpui::px(200.))
                .h(gpui::px(100.))
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .child(gpui::div().h(gpui::px(300.)).w_full())
                .child(
                    gpui::div()
                        .id("target")
                        .track_focus(&self.focus)
                        .w(gpui::px(80.))
                        .h(gpui::px(30.))
                        .inspect("target")
                        .focus_handle(&self.focus),
                )
                .inspect("scroller")
                .scroll_handle(&self.scroll)
                .root(self.store.clone(), 0, WindowId(1))
        }
    }

    #[test]
    fn navigation_reveals_clipped_boundaries_and_focuses_the_real_handle() {
        let mut app = gpui::TestAppContext::single();
        let store = Store::default();
        store.attach();
        let (view, cx) = app.add_window_view(|_, cx| NavigationView {
            store: store.clone(),
            focus: cx.focus_handle(),
            scroll: gpui::ScrollHandle::new(),
        });
        let draw = |cx: &mut gpui::VisualTestContext| {
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            })
        };
        let target = || {
            let snapshot = store.snapshot();
            let element = snapshot
                .elements
                .iter()
                .find(|e| e.semantic_id.as_deref() == Some("target"))
                .unwrap()
                .clone();
            (snapshot.capture, element)
        };
        draw(cx);
        let (generation, before) = target();
        assert!(before.navigation.focus);
        assert!(before.navigation.scroll_into_view);
        assert!(before.layout.unwrap().y >= 300.);
        store
            .navigate(
                &before.id,
                &generation,
                super::super::NavigationAction::ScrollIntoView,
            )
            .unwrap();
        draw(cx);
        let (generation, visible) = target();
        assert!(visible.layout.unwrap().y + visible.layout.unwrap().height <= 100.);
        assert!(matches!(
            store.navigate(&before.id, "old", super::super::NavigationAction::Focus),
            Err(super::super::InspectionError::StaleGeneration)
        ));
        store
            .navigate(
                &visible.id,
                &generation,
                super::super::NavigationAction::Focus,
            )
            .unwrap();
        draw(cx);
        cx.update(|window, cx| assert!(view.read(cx).focus.is_focused(window)));
        assert!(target().1.navigation.focused);
        assert!(
            store.element(&visible.id).is_ok(),
            "focus does not activate an application action"
        );
    }

    struct IdentityView {
        store: Store,
        order: Vec<usize>,
        width: f32,
    }

    impl Render for IdentityView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            gpui::div()
                .children(self.order.iter().map(|&item| {
                    gpui::div()
                        .id(("item", item))
                        .w(gpui::px(self.width))
                        .h(gpui::px(20.))
                        .inspect(format!("item-{item}"))
                }))
                .children((0..2).map(|_| gpui::div().inspect("ambiguous")))
                .inspect("root")
                .root(self.store.clone(), 0, WindowId(1))
        }
    }

    #[test]
    fn temporary_styles_change_real_layout_toggle_undo_and_expire_on_removal() {
        use super::super::{StyleDeclaration, StyleEdit};
        let mut app = gpui::TestAppContext::single();
        let store = Store::default();
        store.attach();
        let (view, cx) = app.add_window_view(|_, _| IdentityView {
            store: store.clone(),
            order: vec![1],
            width: 20.,
        });
        let draw = |cx: &mut gpui::VisualTestContext| {
            cx.update(|window, cx| {
                window.refresh();
                window.draw(cx).clear(cx);
            })
        };
        let target = || {
            store
                .snapshot()
                .elements
                .into_iter()
                .find(|e| e.semantic_id.as_deref() == Some("item-1"))
                .unwrap()
        };
        let edit = |edit| {
            let snapshot = store.snapshot();
            store.edit_style(&target().id, &snapshot.capture, &edit)
        };
        draw(cx);
        let original = target();
        assert!(original.style_editable);
        let declaration = StyleDeclaration {
            property: "width".into(),
            value: "54px".into(),
            enabled: true,
        };
        edit(StyleEdit::Set {
            declaration: declaration.clone(),
        })
        .unwrap();
        draw(cx);
        assert_eq!(target().layout.unwrap().width, 54.);
        assert_eq!(
            target().styles,
            original.styles,
            "authored source refinements stay separate"
        );
        assert_eq!(
            target().style_overrides.as_slice(),
            std::slice::from_ref(&declaration)
        );
        edit(StyleEdit::Set {
            declaration: StyleDeclaration {
                enabled: false,
                ..declaration.clone()
            },
        })
        .unwrap();
        draw(cx);
        assert_eq!(target().layout.unwrap().width, 20.);
        edit(StyleEdit::Undo).unwrap();
        draw(cx);
        assert_eq!(target().layout.unwrap().width, 54.);
        edit(StyleEdit::Reset).unwrap();
        draw(cx);
        assert_eq!(target().layout.unwrap().width, 20.);
        edit(StyleEdit::Undo).unwrap();
        draw(cx);
        assert_eq!(target().layout.unwrap().width, 54.);
        assert!(
            edit(StyleEdit::Set {
                declaration: StyleDeclaration {
                    value: "NaNpx".into(),
                    ..declaration
                }
            })
            .is_err()
        );
        let old = target();
        view.update(cx, |view, cx| {
            view.order.clear();
            cx.notify();
        });
        draw(cx);
        assert!(store.0.borrow().edits.is_empty());
        assert!(
            store
                .edit_style(&old.id, &store.snapshot().capture, &StyleEdit::Reset)
                .is_err()
        );
    }

    #[test]
    fn presentation_identities_follow_keyed_items_resize_and_consecutive_lifetimes() {
        let mut app = gpui::TestAppContext::single();
        let store = Store::default();
        store.attach();
        let (view, cx) = app.add_window_view(|_, _| IdentityView {
            store: store.clone(),
            order: vec![1, 2],
            width: 20.,
        });
        let draw = |cx: &mut gpui::VisualTestContext| {
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            })
        };
        let identity = |name: &str| {
            store
                .snapshot()
                .elements
                .into_iter()
                .find(|element| element.semantic_id.as_deref() == Some(name))
                .unwrap()
        };
        draw(cx);
        let one = identity("item-1");
        let two = identity("item-2");
        let root = identity("root");
        assert!(one.stable_id.is_some());
        assert_ne!(one.stable_id, two.stable_id);
        assert!(
            store
                .snapshot()
                .elements
                .iter()
                .filter(|e| e.semantic_id.as_deref() == Some("ambiguous"))
                .all(|e| e.stable_id.is_none())
        );

        store
            .highlight(Some(&one.id), &store.snapshot().capture)
            .unwrap();

        view.update(cx, |view, cx| {
            view.order = vec![2, 3, 1];
            view.width = 42.;
            cx.notify();
        });
        draw(cx);
        assert_eq!(identity("item-1").stable_id, one.stable_id);
        assert_eq!(identity("item-2").stable_id, two.stable_id);
        assert_eq!(identity("root").stable_id, root.stable_id);
        assert_ne!(
            identity("item-1").id,
            one.id,
            "control IDs remain capture scoped"
        );
        assert_eq!(identity("item-1").layout.unwrap().width, 42.);
        assert_eq!(
            store.0.borrow().windows[&WindowId(1)]
                .highlight
                .as_ref()
                .unwrap()
                .0
                .layout
                .unwrap()
                .width,
            42.
        );

        view.update(cx, |view, cx| {
            view.order = vec![2];
            cx.notify();
        });
        draw(cx);
        view.update(cx, |view, cx| {
            view.order = vec![1, 2];
            cx.notify();
        });
        draw(cx);
        assert_ne!(
            identity("item-1").stable_id,
            one.stable_id,
            "unmounted boundaries must expire"
        );
        assert_eq!(identity("item-2").stable_id, two.stable_id);
    }

    #[test]
    fn length_resolution_preserves_literal_font_names_and_percentages() {
        let mut value = serde_json::json!({"text":{"font_family":"2rem"}, "padding":{"top":"0.5rem","left":"50%"}});

        resolve_lengths(&mut value, 16.);
        assert_eq!(value["text"]["font_family"], "2rem");
        assert_eq!(value["padding"]["top"], "8px");
        assert_eq!(value["padding"]["left"], "50%");
    }

    struct TestView {
        store: Store,
        clicks: Rc<Cell<u32>>,
    }

    impl Render for TestView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let clicks = self.clicks.clone();

            gpui::div()
                .id("button")
                .size_full()
                .on_mouse_down(gpui::MouseButton::Left, move |_, _, _| {
                    clicks.set(clicks.get() + 1)
                })
                .child(
                    gpui::div()
                        .text_color(gpui::rgb(0xabcdef))
                        .text_size(gpui::rems(1.5))
                        .child(
                            gpui::div()
                                .absolute()
                                .left(gpui::px(80.))
                                .top(gpui::px(120.))
                                .w(gpui::px(100.))
                                .h(gpui::px(40.))
                                .bg(gpui::rgb(0x29424b))
                                .hover(|style| style.bg(gpui::rgb(0x365849)))
                                .inspect("offset-button"),
                        ),
                )
                .inspect("button")
                .root(self.store.clone(), 0, WindowId(1))
        }
    }

    #[test]
    fn resolved_styles_follow_real_hover_and_uninstrumented_text_inheritance() {
        let mut app = gpui::TestAppContext::single();
        let store = Store::default();

        store.attach();
        let (_, cx) = app.add_window_view(|_, _| TestView {
            store: store.clone(),
            clicks: Rc::new(Cell::new(0)),
        });

        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        let before = store.snapshot();

        let button = |snapshot: &Snapshot| {
            snapshot
                .elements
                .iter()
                .find(|element| element.semantic_id.as_deref() == Some("offset-button"))
                .unwrap()
                .clone()
        };

        let initial = button(&before);
        let resolved = initial.resolved.as_ref().unwrap();

        assert!(!resolved.hovered);
        assert_eq!(
            super::super::styles::color(&resolved.properties["background"])
                .unwrap()
                .css(),
            "#29424b"
        );
        assert_eq!(
            super::super::styles::color(&resolved.properties["text"]["color"])
                .unwrap()
                .css(),
            "#abcdef"
        );
        assert!(
            resolved.properties["text"]["font_size"]
                .as_str()
                .unwrap()
                .ends_with("px")
        );
        assert_eq!(resolved.properties["size"]["width"], "100px");
        let inherited = resolved
            .layers
            .iter()
            .find(|layer| layer.origin == super::super::StyleOrigin::Inherited)
            .unwrap();
        assert_eq!(
            inherited.properties["text"]["color"],
            resolved.properties["text"]["color"]
        );
        assert!(
            resolved
                .layers
                .iter()
                .any(|layer| layer.origin == super::super::StyleOrigin::Authored
                    && layer.properties.get("background").is_some())
        );
        let moving = |cx: &mut gpui::VisualTestContext, x, y| {
            cx.simulate_event(gpui::MouseMoveEvent {
                position: gpui::point(gpui::px(x), gpui::px(y)),
                pressed_button: None,
                modifiers: Default::default(),
            });
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            });
        };

        moving(cx, 100., 140.);
        let hovered = store.snapshot();
        let hovered_button = button(&hovered);
        let resolved = hovered_button.resolved.as_ref().unwrap();

        assert!(resolved.hovered);
        assert!(resolved.layers.iter().any(|layer| layer.origin
            == super::super::StyleOrigin::Interaction
            && layer.properties.get("background").is_some()));
        assert_eq!(
            super::super::styles::color(&resolved.properties["background"])
                .unwrap()
                .css(),
            "#365849"
        );
        assert_eq!(
            hovered.capture, before.capture,
            "style changes must keep selection IDs stable"
        );
        assert_eq!(hovered_button.id, initial.id);
        assert_eq!(
            hovered_button.styles, initial.styles,
            "authored styles remain separate"
        );
        moving(cx, 5., 5.);
        let after = button(&store.snapshot());

        assert!(!after.resolved.as_ref().unwrap().hovered);
        assert_eq!(
            super::super::styles::color(&after.resolved.unwrap().properties["background"])
                .unwrap()
                .css(),
            "#29424b"
        );
    }

    #[test]
    fn captured_content_box_matches_real_gpui_child_layout() {
        struct Boxes(Store);
        impl Render for Boxes {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                gpui::div()
                    .flex()
                    .flex_col()
                    .size_full()
                    .p(gpui::px(10.))
                    .child(
                        gpui::div()
                            .flex()
                            .w(gpui::px(120.))
                            .h(gpui::px(90.))
                            .border_2()
                            .p(gpui::px(8.))
                            .m(gpui::px(5.))
                            .child(gpui::div().size_full().inspect("content"))
                            .inspect("outer"),
                    )
                    .inspect("root")
                    .root(self.0.clone(), 0, WindowId(1))
            }
        }
        let mut app = gpui::TestAppContext::single();
        let store = Store::default();
        store.attach();
        let (_, cx) = app.add_window_view(|_, _| Boxes(store.clone()));
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        let snapshot = store.snapshot();
        let find = |id| {
            snapshot
                .elements
                .iter()
                .find(|element| element.semantic_id.as_deref() == Some(id))
                .unwrap()
        };
        let boxes = find("outer").box_model.as_ref().unwrap();
        assert_eq!(boxes.border, find("outer").layout.unwrap());
        assert_eq!(boxes.content, find("content").layout);
        let margin = boxes.margin.unwrap();
        assert_eq!(margin.x, boxes.border.x - 5.);
        assert_eq!(margin.width, boxes.border.width + 10.);
    }

    #[test]
    fn hover_highlight_uses_the_picked_elements_window_position() {
        let mut app = gpui::TestAppContext::single();
        let store = Store::default();

        store.attach();
        let (_, cx) = app.add_window_view(|_, _| TestView {
            store: store.clone(),
            clicks: Rc::new(Cell::new(0)),
        });

        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        store.inspect_mode(true, 1000).unwrap();
        store.pointer(WindowId(1), 100., 140., false);
        let picked = store
            .element(store.pick().unwrap().hovered.as_deref().unwrap())
            .unwrap();

        assert_eq!(picked.semantic_id.as_deref(), Some("offset-button"));
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        let bounds = cx.debug_bounds("rake.inspection.highlight").unwrap();

        assert_eq!(bounds.origin, gpui::point(gpui::px(80.), gpui::px(120.)));
        assert_eq!(bounds.size, gpui::size(gpui::px(100.), gpui::px(40.)));
    }

    #[test]
    fn picker_intercepts_native_dispatch_and_releases_on_expiry_and_escape() {
        let mut cx = gpui::TestAppContext::single();
        let store = Store::default();

        store.attach();
        let clicks = Rc::new(Cell::new(0));

        let window = cx.add_window(|_, _| TestView {
            store: store.clone(),
            clicks: clicks.clone(),
        });

        cx.update_window(window.into(), |_, window, cx| {
            window.draw(cx).clear(cx);
        })
        .unwrap();
        store.inspect_mode(true, 1000).unwrap();
        let click = |cx: &mut gpui::TestAppContext| {
            cx.update_window(window.into(), |_, window, cx| {
                let position = gpui::point(gpui::px(5.), gpui::px(5.));

                window.dispatch_event(
                    gpui::MouseMoveEvent {
                        position,
                        pressed_button: None,
                        modifiers: Default::default(),
                    }
                    .to_platform_input(),
                    cx,
                );
                window.dispatch_event(
                    gpui::MouseDownEvent {
                        position,
                        button: gpui::MouseButton::Left,
                        modifiers: Default::default(),
                        click_count: 1,
                        first_mouse: false,
                    }
                    .to_platform_input(),
                    cx,
                );
                window.dispatch_event(
                    gpui::MouseUpEvent {
                        position,
                        button: gpui::MouseButton::Left,
                        modifiers: Default::default(),
                        click_count: 1,
                    }
                    .to_platform_input(),
                    cx,
                );
            })
            .unwrap();
        };

        click(&mut cx);
        assert_eq!(clicks.get(), 0);
        assert_eq!(store.pick().unwrap().pick_sequence, 1);
        assert!(!store.pick().unwrap().mode);
        store.inspect_mode(true, 1000).unwrap();
        store.0.borrow_mut().mode_until =
            Some(web_time::Instant::now() - std::time::Duration::from_millis(1));
        click(&mut cx);
        assert_eq!(clicks.get(), 1);
        store.inspect_mode(true, 1000).unwrap();
        cx.update_window(window.into(), |_, window, cx| {
            window.dispatch_event(
                gpui::KeyDownEvent {
                    keystroke: gpui::Keystroke::parse("escape").unwrap(),
                    is_held: false,
                    prefer_character_input: false,
                }
                .to_platform_input(),
                cx,
            );
        })
        .unwrap();
        assert!(!store.pick().unwrap().mode);
        click(&mut cx);
        assert_eq!(clicks.get(), 2);
    }
}

use super::{bound_resolved_styles, retain};
