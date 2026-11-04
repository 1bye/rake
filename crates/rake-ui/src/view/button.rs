//! Inspectable Rake button over Base interaction, focus and disabled semantics.

use gpui::{
    App, Bounds, Div, Element, ElementId, FocusHandle, GlobalElementId, InspectorElementId,
    InteractiveElement, Interactivity, IntoElement, LayoutId, ParentElement, Pixels, RenderOnce,
    Stateful, StyleRefinement, Styled, Window, accesskit,
};

/// Completes a macro-authored button with managed focus and accessible disabled state.
#[doc(hidden)]
pub fn button(
    mut button: Button,
    disabled: bool,
    focus: Option<FocusHandle>,
    tabindex: Option<isize>,
) -> Button {
    let mut inner = button
        .inner
        .take()
        .expect("button configured before layout");

    if disabled {
        inner = inner
            .capture_any_mouse_down(|_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .capture_any_mouse_up(|_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .capture_key_down(|event, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space")
                    && !event.keystroke.modifiers.modified()
                {
                    window.prevent_default();
                    cx.stop_propagation();
                }
            })
            .capture_key_up(|event, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space")
                    && !event.keystroke.modifiers.modified()
                {
                    window.prevent_default();
                    cx.stop_propagation();
                }
            });
    }

    button.inner = Some(inner);
    button.disabled = disabled;
    button.focus = focus;
    button.tabindex = tabindex.unwrap_or(0);

    button
}

/// Begins a macro-authored button before attributes and children are applied.
#[doc(hidden)]
#[track_caller]
pub fn button_base(id: impl Into<ElementId>) -> Button {
    let inner = gpui::div().id(id);

    Button {
        id: Element::id(&inner),
        source: inner.source_location(),
        inner: Some(inner),
        children: Vec::new(),
        rendered: None,
        authored: StyleRefinement::default(),
        live: None,
        disabled: false,
        focus: None,
        tabindex: 0,
    }
}

/// A directly rendered GPUI button with stable focus and disabled semantics.
#[doc(hidden)]
pub struct Button {
    id: Option<ElementId>,
    source: Option<&'static std::panic::Location<'static>>,
    inner: Option<Stateful<Div>>,
    children: Vec<gpui::AnyElement>,
    rendered: Option<Box<dyn RenderedButton>>,
    authored: StyleRefinement,
    live: Option<gpui::Style>,
    disabled: bool,
    focus: Option<FocusHandle>,
    tabindex: isize,
}

impl Button {
    /// Current real focus handle for development inspection, after request-layout.
    #[doc(hidden)]
    pub fn inspection_focus(&self) -> Option<&FocusHandle> {
        self.focus.as_ref()
    }

    /// Interaction before Base consumes its builder during layout.
    #[doc(hidden)]
    pub fn inspection_interactivity(&mut self) -> Option<&mut Interactivity> {
        self.inner.as_mut().map(|inner| inner.interactivity())
    }

    /// Resolved style used to lay out Base's button in this frame.
    #[doc(hidden)]
    pub fn inspection_style(&self) -> Option<&gpui::Style> {
        self.live.as_ref()
    }
}

struct ButtonFocus(FocusHandle);

// GPUI exposes fluent focus configuration but no mutable Interactivity setter.
struct Interaction<'a>(&'a mut Interactivity);

impl InteractiveElement for Interaction<'_> {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.0
    }
}

impl InteractiveElement for Button {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.inner
            .as_mut()
            .expect("button builder is configured before layout")
            .interactivity()
    }
}

impl gpui::StatefulInteractiveElement for Button {}

impl Styled for Button {
    fn style(&mut self) -> &mut StyleRefinement {
        self.inner
            .as_mut()
            .map_or(&mut self.authored, |inner| inner.style())
    }
}

impl ParentElement for Button {
    fn extend(&mut self, elements: impl IntoIterator<Item = gpui::AnyElement>) {
        self.children.extend(elements);
    }
}

impl IntoElement for Button {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

// Base deliberately hides its rendered type. Erase only this button's layout
// state, forwarding Element hooks directly rather than adding another GPUI node.
trait RenderedButton {
    fn role(&self) -> Option<accesskit::Role>;

    fn info(&self, node: &mut accesskit::Node);

    fn synthetic(&mut self, state: &mut dyn std::any::Any, builder: &mut gpui::A11ySubtreeBuilder);

    fn layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Box<dyn std::any::Any>);

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut dyn std::any::Any,
        window: &mut Window,
        cx: &mut App,
    ) -> Box<dyn std::any::Any>;

    // Mirrors GPUI Element::paint to preserve its lifecycle and native hooks.
    #[allow(clippy::too_many_arguments)]
    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut dyn std::any::Any,
        prepaint: &mut dyn std::any::Any,
        window: &mut Window,
        cx: &mut App,
    );
}

struct Rendered<E>(E);

impl<E: Element> RenderedButton for Rendered<E> {
    fn role(&self) -> Option<accesskit::Role> {
        self.0.a11y_role()
    }

    fn info(&self, node: &mut accesskit::Node) {
        self.0.write_a11y_info(node);
    }

    fn synthetic(&mut self, state: &mut dyn std::any::Any, builder: &mut gpui::A11ySubtreeBuilder) {
        self.0.a11y_synthetic_children(
            state.downcast_mut().expect("button prepaint state"),
            builder,
        );
    }

    fn layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Box<dyn std::any::Any>) {
        let (layout, state) = self.0.request_layout(id, inspector, window, cx);

        (layout, Box::new(state))
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut dyn std::any::Any,
        window: &mut Window,
        cx: &mut App,
    ) -> Box<dyn std::any::Any> {
        Box::new(self.0.prepaint(
            id,
            inspector,
            bounds,
            state.downcast_mut().expect("button layout state"),
            window,
            cx,
        ))
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut dyn std::any::Any,
        prepaint: &mut dyn std::any::Any,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.0.paint(
            id,
            inspector,
            bounds,
            state.downcast_mut().expect("button layout state"),
            prepaint.downcast_mut().expect("button prepaint state"),
            window,
            cx,
        );
    }
}

impl Element for Button {
    type RequestLayoutState = Box<dyn std::any::Any>;
    type PrepaintState = Box<dyn std::any::Any>;

    fn id(&self) -> Option<ElementId> {
        self.id.clone()
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.source
    }

    fn a11y_role(&self) -> Option<accesskit::Role> {
        self.rendered.as_ref().map_or_else(
            || self.inner.as_ref().and_then(Element::a11y_role),
            |inner| inner.role(),
        )
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        if let Some(inner) = &self.rendered {
            inner.info(node);
        } else if let Some(inner) = &self.inner {
            inner.write_a11y_info(node);
        }

        if self.disabled {
            node.set_disabled();
            node.clear_actions();
        }
    }

    fn a11y_synthetic_children(
        &mut self,
        prepaint: &mut Self::PrepaintState,
        builder: &mut gpui::A11ySubtreeBuilder,
    ) {
        if let Some(inner) = &mut self.rendered {
            inner.synthetic(prepaint.as_mut(), builder);
        }
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        crate::init(cx);
        let mut inner = self.inner.take().expect("button layouts once per frame");
        let explicit = self.focus.clone();

        let focus = window.with_element_state(
            id.expect("macro buttons have IDs"),
            |state: Option<ButtonFocus>, _| {
                let focus = explicit
                    .or_else(|| state.map(|state| state.0))
                    .unwrap_or_else(|| cx.focus_handle());

                (focus.clone(), ButtonFocus(focus))
            },
        );

        Interaction(inner.interactivity()).track_focus(&focus);
        self.authored = inner.style().clone();
        self.live = Some(inner.interactivity().compute_style(id, None, window, cx));
        let mut base = gpui_base::Button::new(Element::id(&inner).expect("macro button ID"))
            .disabled(self.disabled)
            .role(inner.a11y_role().unwrap_or(accesskit::Role::Button))
            .track_focus(&focus)
            .tab_index(self.tabindex)
            .tab_stop(self.tabindex >= 0);

        std::mem::swap(base.interactivity(), inner.interactivity());
        *base.style() = self.authored.clone();

        // Preserve Rake's inherited line height instead of Base's neutral default.
        if base.style().text.line_height.is_none() {
            base = base.line_height(window.text_style().line_height);
        }

        // Children are already on the input div; move its content separately.
        // The macro appends them to Button before request-layout.
        base.extend(std::mem::take(&mut self.children));
        self.focus = Some(focus);
        let element = base.render(window, cx).into_element();
        let mut rendered: Box<dyn RenderedButton> = Box::new(Rendered(element));
        let layout = rendered.layout(id, inspector, window, cx);

        self.rendered = Some(rendered);

        layout
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
        self.rendered.as_mut().expect("button laid out").prepaint(
            id,
            inspector,
            bounds,
            state.as_mut(),
            window,
            cx,
        )
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
        self.rendered.as_mut().expect("button laid out").paint(
            id,
            inspector,
            bounds,
            state.as_mut(),
            prepaint.as_mut(),
            window,
            cx,
        );
    }
}
