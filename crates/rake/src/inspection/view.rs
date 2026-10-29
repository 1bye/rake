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
                resolved: None,
                metadata_truncated: truncated,
                tab_id: owner.map(|owner| owner.0),
                entry_id: owner.map(|owner| owner.1),
            };

            let index = capture.snapshot.elements.len();
            let retained = retain(&mut capture.snapshot, element, &mut capture.bytes);

            retained_index = retained.as_ref().map(|_| index);
            let parent = retained.or_else(|| capture.parents.last().cloned().flatten());

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
            }))
        });

        if let Some(index) = prepaint.2
            && cx.global::<ActiveCapture>().0.is_some()
        {
            let resolved = resolved_style(&mut self.inner, &prepaint.0, id, bounds, window, cx);

            if let Some(element) = cx
                .global_mut::<ActiveCapture>()
                .0
                .as_mut()
                .and_then(|capture| capture.snapshot.elements.get_mut(index))
            {
                element.resolved = resolved;
            }
        }

        if let Some((store, _, host)) = &self.root {
            listeners(store, *host, window, cx);
        }

        self.inner
            .paint(id, inspector_id, bounds, state, &mut prepaint.0, window, cx);

        if let Some(previous) = previous {
            let completed = std::mem::replace(&mut cx.global_mut::<ActiveCapture>().0, previous);

            if let (Some((store, _, host)), Some(mut completed)) = (&self.root, completed) {
                bound_resolved_styles(&mut completed.snapshot);
                store.publish(*host, completed.snapshot);
            }
        }

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

fn resolved_style<E: gpui::Element>(
    inner: &mut E,
    prepaint: &E::PrepaintState,
    id: Option<&GlobalElementId>,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) -> Option<super::ResolvedStyle> {
    use gpui::{InteractiveElement, Refineable};
    use std::any::Any;
    let hitbox = (prepaint as &dyn Any)
        .downcast_ref::<Option<gpui::Hitbox>>()?
        .as_ref();

    let inner = inner as &mut dyn Any;

    let interactive = if inner.is::<gpui::Div>() {
        inner.downcast_mut::<gpui::Div>()?.interactivity()
    } else {
        inner
            .downcast_mut::<gpui::Stateful<gpui::Div>>()?
            .interactivity()
    };

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

    Some(super::ResolvedStyle {
        properties,
        hovered: hitbox.is_some_and(|hitbox| hitbox.is_hovered(window)),
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

    let label = format!(
        "{} {} · {}×{}",
        super::display_name(&element.kind),
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
    element.prepaint_at(point(px(layout.x), px(layout.y)), window, cx);

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
