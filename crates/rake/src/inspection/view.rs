//! GPUI adapter: transparent delegation preserves layout, IDs and accessibility.

// shortcut: opaque/cached GPUI descendants need a future public full-tree capture hook.

use std::panic::Location;

use gpui::{
    App, Bounds, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId, Pixels,
    Styled, Window,
};

use super::{Element, Layout, Snapshot, SourceLocation, Status, Store};
use crate::router::{EntryId, TabId, WindowId};

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
    root: Option<(Store, u64, WindowId)>,
    owner: Option<(TabId, EntryId)>,
}

impl<E: gpui::Element> Inspected<E> {
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
    type PrepaintState = (E::PrepaintState, Option<gpui::AnyElement>);

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

        let previous = self.root.as_ref().map(|(store, revision, _)| {
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

            if let (Some((store, _, host)), Some(completed)) = (&self.root, completed) {
                store.publish(*host, completed.snapshot);
            }
        }

        let overlay = self
            .root
            .as_ref()
            .and_then(|(store, _, host)| overlay(store, *host, window, cx));
        (result, overlay)
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
        if let Some((store, _, host)) = &self.root {
            listeners(store, *host, window, cx);
        }
        self.inner
            .paint(id, inspector_id, bounds, state, &mut prepaint.0, window, cx);
        if let Some(overlay) = &mut prepaint.1 {
            overlay.paint(window, cx);
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
    let label = format!(
        "{} {} · {}×{}",
        element.kind,
        element.semantic_id.as_deref().unwrap_or(""),
        layout.width,
        layout.height
    );
    drop(state);
    let mut element = div()
        .absolute()
        .left(px(layout.x))
        .top(px(layout.y))
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
    element.prepaint_at(point(px(0.), px(0.)), window, cx);
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
                .inspect("button")
                .root(self.store.clone(), 0, WindowId(1))
        }
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
