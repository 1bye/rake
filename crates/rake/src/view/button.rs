//! Button focus ownership and disabled semantics over the pinned GPUI div element.

use gpui::{
    App, Bounds, Div, Element, ElementId, FocusHandle, GlobalElementId, InspectorElementId,
    InteractiveElement, Interactivity, IntoElement, LayoutId, ParentElement, Pixels, Stateful,
    StyleRefinement, Styled, Window, accesskit,
};

/// Completes a macro-authored button with managed focus and accessible disabled state.
#[doc(hidden)]
pub fn button(
    mut inner: Stateful<Div>,
    disabled: bool,
    focus: Option<FocusHandle>,
    tabindex: Option<isize>,
) -> Button {
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

    Button {
        inner,
        disabled,
        focus,
        tabindex: tabindex.unwrap_or(0),
    }
}

/// A directly rendered GPUI button with stable focus and disabled semantics.
#[doc(hidden)]
pub struct Button {
    inner: Stateful<Div>,
    disabled: bool,
    focus: Option<FocusHandle>,
    tabindex: isize,
}

struct ButtonFocus(FocusHandle);

// GPUI exposes fluent focus configuration but no mutable Interactivity setter.
struct Interaction<'a>(&'a mut Interactivity);

impl InteractiveElement for Interaction<'_> {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.0
    }
}

impl Styled for Button {
    fn style(&mut self) -> &mut StyleRefinement {
        self.inner.style()
    }
}

impl ParentElement for Button {
    fn extend(&mut self, elements: impl IntoIterator<Item = gpui::AnyElement>) {
        self.inner.extend(elements);
    }
}

impl IntoElement for Button {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Button {
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

        if self.disabled {
            node.set_disabled();
            node.clear_actions();
        } else {
            node.add_action(accesskit::Action::Focus);
        }
    }

    fn a11y_synthetic_children(
        &mut self,
        prepaint: &mut Self::PrepaintState,
        builder: &mut gpui::A11ySubtreeBuilder,
    ) {
        self.inner.a11y_synthetic_children(prepaint, builder);
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let explicit = self.focus.clone();

        let focus = window.with_element_state(
            id.expect("macro buttons have IDs"),
            |state: Option<ButtonFocus>, _| {
                let focus = explicit
                    .or_else(|| state.map(|state| state.0))
                    .unwrap_or_else(|| cx.focus_handle())
                    .tab_index(self.tabindex)
                    .tab_stop(!self.disabled && self.tabindex >= 0);

                (focus.clone(), ButtonFocus(focus))
            },
        );

        // Keep an already-focused disabled button as a dispatch anchor so Tab
        // and enclosing actions still work; it is neither clickable nor a tab stop.
        Interaction(self.inner.interactivity()).track_focus(&focus);
        self.focus = Some(focus);

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
        self.inner
            .prepaint(id, inspector_id, bounds, state, window, cx)
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
}
