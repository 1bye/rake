//! Component surfaces and prepaint hooks; anchors are measured before deferred layers.

use gpui::{
    self, AnyElement, App, AvailableSpace, Bounds, Div, Element, ElementId, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, Pixels, RenderOnce, SharedString, Stateful, Window,
    accesskit, point, prelude::*, px, rgb, size,
};

use super::state::{AnchorState, Kind, OverlayHostState, OverlayState};

/// Window-root host for overlay controls, with background actions isolated in `body`.
#[derive(IntoElement)]
pub struct OverlayHost {
    /// State created once in the window's owning entity.
    pub state: OverlayHostState,
    /// The normal application view. Put background key contexts/action handlers here.
    pub body: AnyElement,
    /// Popovers and dialogs, normally authored as nested component tags.
    pub children: Vec<AnyElement>,
}

impl RenderOnce for OverlayHost {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let host = self.state.0.borrow();

        let fallback = gpui::div()
            .id("rake::overlay:fallback")
            .size_0()
            .track_focus(&host.fallback);

        let background = Frame {
            inner: gpui::div()
                .id("rake::overlay:body")
                .size_full()
                .role(accesskit::Role::Group)
                .child(self.body),
            hook: Hook::Background(self.state.is_modal()),
        };

        let root = gpui::div()
            .id("rake::overlay:host")
            .relative()
            .size_full()
            .track_focus(&host.root)
            .key_context("RakeOverlayHost")
            .on_key_down(crate::view::tab_navigation)
            .child(fallback)
            .child(background)
            .children(self.children);

        drop(host);

        Frame {
            inner: root,
            hook: Hook::Host(self.state),
        }
    }
}

/// Tracks the current visible window bounds of its trigger content.
#[derive(IntoElement)]
pub struct OverlayAnchor {
    /// A handle created through the same host as the corresponding popover.
    pub state: AnchorState,
    /// Trigger content, commonly a button with an accessible expanded state.
    pub children: Vec<AnyElement>,
}

impl RenderOnce for OverlayAnchor {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let id = self.state.inner.borrow().id;

        Frame {
            inner: gpui::div()
                .id(("rake::overlay:anchor", id))
                .flex()
                .flex_col()
                .children(self.children),
            hook: Hook::Anchor(self.state),
        }
    }
}

/// An anchored, nonmodal group of controls with Escape and outside-click dismissal.
/// Use a state created by [`OverlayHostState::popover`].
#[derive(IntoElement)]
pub struct Popover {
    /// Retained interaction state.
    pub state: OverlayState,
    /// Live trigger geometry measured by an [`OverlayAnchor`].
    pub anchor: AnchorState,
    /// Popup content. Menu roles and arrow-key behavior are application-defined.
    pub children: Vec<AnyElement>,
}

impl RenderOnce for Popover {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        assert_eq!(
            self.state.kind(),
            Kind::Popover,
            "Popover requires popover state"
        );
        assert!(
            std::rc::Rc::ptr_eq(&self.state.host.0, &self.anchor.host.0),
            "popover and anchor must share a host"
        );
        self.state.inner.borrow_mut().anchor = Some(self.anchor);

        floating(self.state, None, None, self.children)
    }
}

/// A modal dialog with initial focus, contained Tab navigation and focus restoration.
/// Use a state created by [`OverlayHostState::dialog`].
#[derive(IntoElement)]
pub struct Dialog {
    /// Retained interaction state.
    pub state: OverlayState,
    /// The visible heading and accessible name.
    pub title: SharedString,
    /// Optional footer controls; use `None` explicitly when absent.
    pub footer: Option<AnyElement>,
    /// Dialog content; retained GPUI entities can be mounted here.
    pub children: Vec<AnyElement>,
}

impl RenderOnce for Dialog {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        assert_eq!(
            self.state.kind(),
            Kind::Dialog,
            "Dialog requires dialog state"
        );

        floating(self.state, Some(self.title), self.footer, self.children)
    }
}

fn floating(
    state: OverlayState,
    title: Option<SharedString>,
    footer: Option<AnyElement>,
    children: Vec<AnyElement>,
) -> AnyElement {
    if !state.is_open() {
        return gpui::Empty.into_any_element();
    }

    let priority = state.priority();

    gpui::deferred(Floating {
        state,
        title,
        footer,
        children: Some(children),
        painted: None,
    })
    .with_priority(priority)
    .into_any_element()
}

struct Floating {
    state: OverlayState,
    title: Option<SharedString>,
    footer: Option<AnyElement>,
    children: Option<Vec<AnyElement>>,
    painted: Option<AnyElement>,
}

impl IntoElement for Floating {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Floating {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (
            window.request_layout(
                gpui::Style {
                    position: gpui::Position::Absolute,
                    ..Default::default()
                },
                [],
                cx,
            ),
            (),
        )
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if !self.state.is_open() {
            return;
        }

        let anchor = self.state.inner.borrow().anchor.clone();

        let position = if let Some(anchor) = anchor {
            let data = anchor.inner.borrow();

            if data.frame != self.state.host.0.borrow().frame {
                return;
            }

            let Some(bounds) = data.bounds else {
                return;
            };

            Some(bounds.bottom_left())
        } else {
            None
        };

        let viewport = window.viewport_size();
        let modal = self.state.modal_context();

        let (id, root, first, last) = {
            let data = self.state.inner.borrow();

            (
                data.id,
                data.root.clone(),
                data.first.clone(),
                data.last.clone(),
            )
        };

        let key_state = self.state.clone();

        let mut surface = gpui::div()
            .id(("rake::overlay:surface", id))
            .track_focus(&root)
            .key_context("RakeOverlay")
            .role(if self.title.is_some() {
                accesskit::Role::Dialog
            } else {
                accesskit::Role::Group
            })
            .w(px(if self.title.is_some() { 520. } else { 320. }))
            .max_w((viewport.width - px(32.)).max(px(1.)))
            .max_h((viewport.height - px(32.)).max(px(1.)))
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .rounded_lg()
            .shadow_lg()
            .bg(rgb(0x22323d))
            .text_color(rgb(0xf0f4e8))
            .overflow_y_scroll()
            .occlude()
            .on_key_down(move |event, window, cx| {
                if !key_state.is_topmost() || window.default_prevented() {
                    return;
                }

                let mut modifiers = event.keystroke.modifiers;

                modifiers.shift = false;

                if event.keystroke.key == "escape" && !event.keystroke.modifiers.modified() {
                    key_state.close(window, cx);
                } else if event.keystroke.key == "tab" && !modifiers.modified() {
                    key_state.navigate(event.keystroke.modifiers.shift, window, cx);
                } else {
                    return;
                }

                window.prevent_default();
                cx.stop_propagation();
            })
            .child(gpui::div().id("start").size_0().track_focus(&first));

        if let Some(title) = self.title.take() {
            surface = surface.aria_label(title.clone()).child(
                gpui::div()
                    .text_lg()
                    .font_weight(gpui::FontWeight::BOLD)
                    .child(title),
            );
        }

        surface = surface
            .children(
                self.children
                    .take()
                    .expect("floating content prepaints once"),
            )
            .children(self.footer.take())
            .child(gpui::div().id("end").size_0().track_focus(&last));
        let surface = Frame {
            inner: surface,
            hook: Hook::Surface(self.state.clone(), modal),
        };

        let backdrop_state = self.state.clone();

        let backdrop = gpui::div()
            .id(("rake::overlay:backdrop", id))
            .absolute()
            .inset_0()
            .occlude()
            .when(modal, |s| s.bg(gpui::rgba(0x00000066)))
            .on_any_mouse_down(move |_, window, cx| {
                if backdrop_state.is_topmost() {
                    backdrop_state.close(window, cx);
                }

                window.prevent_default();
                cx.stop_propagation();
            })
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation());

        let positioned = if let Some(position) = position {
            gpui::anchored()
                .position(position)
                .anchor(gpui::Anchor::TopLeft)
                .snap_to_window_with_margin(gpui::Edges::all(px(16.)))
                .child(surface)
                .into_any_element()
        } else {
            gpui::div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .child(surface)
                .into_any_element()
        };

        let mut screen = gpui::div()
            .absolute()
            .left_0()
            .top_0()
            .w(viewport.width)
            .h(viewport.height)
            .child(backdrop)
            .child(positioned)
            .into_any_element();

        screen.prepaint_as_root(
            point(px(0.), px(0.)),
            size(
                AvailableSpace::Definite(viewport.width),
                AvailableSpace::Definite(viewport.height),
            ),
            window,
            cx,
        );
        self.painted = Some(screen);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(screen) = &mut self.painted {
            screen.paint(window, cx);
        }
    }
}

enum Hook {
    Host(OverlayHostState),
    Anchor(AnchorState),
    Surface(OverlayState, bool),
    Background(bool),
}

struct Frame {
    inner: Stateful<Div>,
    hook: Hook,
}

impl IntoElement for Frame {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Frame {
    type RequestLayoutState = <Stateful<Div> as Element>::RequestLayoutState;
    type PrepaintState = <Stateful<Div> as Element>::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        Element::id(&self.inner)
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.inner.source_location()
    }

    fn a11y_role(&self) -> Option<accesskit::Role> {
        self.inner.a11y_role()
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        self.inner.write_a11y_info(node);

        match &self.hook {
            Hook::Background(true) => node.set_hidden(),
            Hook::Surface(state, _) if !state.is_topmost() && state.host.is_modal() => {
                node.set_hidden();
            }
            Hook::Surface(state, true) if state.is_topmost() => node.set_modal(),
            _ => {}
        }
    }

    fn a11y_synthetic_children(
        &mut self,
        prepaint: &mut Self::PrepaintState,
        builder: &mut gpui::A11ySubtreeBuilder,
    ) {
        Element::a11y_synthetic_children(&mut self.inner, prepaint, builder);
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.inner.request_layout(id, inspector, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        if let Hook::Host(host) = &self.hook {
            host.begin_frame(window);
        }

        let hitbox = self
            .inner
            .prepaint(id, inspector, bounds, state, window, cx);

        match &self.hook {
            Hook::Host(host) => {
                let host = host.clone();
                let handle = window.window_handle();

                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, cx| host.audit(window, cx));
                });
            }
            Hook::Anchor(anchor) => {
                let visible = bounds
                    .intersect(&window.content_mask().bounds)
                    .intersect(&Bounds {
                        origin: point(px(0.), px(0.)),
                        size: window.viewport_size(),
                    });

                let mut data = anchor.inner.borrow_mut();

                data.frame = anchor.host.0.borrow().frame;
                data.bounds = (visible.size.width > px(0.) && visible.size.height > px(0.))
                    .then_some(visible);
            }
            Hook::Surface(overlay, _) => {
                let mut data = overlay.inner.borrow_mut();

                data.frame = overlay.host.0.borrow().frame;
                data.bounds = Some(bounds);
            }
            _ => {}
        }

        hitbox
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner
            .paint(id, inspector, bounds, state, prepaint, window, cx);
    }
}
