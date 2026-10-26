//! Accessible text conversion and image delegation for experimental views.

mod button;

pub use button::button;

use std::any::Any;
#[cfg(debug_assertions)]
use std::{collections::HashMap, panic::Location};

use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, Img, InspectorElementId, InteractiveElement,
    Interactivity, IntoElement, LayoutId, ParentElement, Pixels, SharedString, Stateful,
    StatefulInteractiveElement, StyleRefinement, Styled, StyledImage, Window, accesskit,
};

/// Checks direct child identities within one macro-authored GPUI parent.
#[doc(hidden)]
#[derive(Default)]
pub struct Children {
    #[cfg(debug_assertions)]
    ids: HashMap<ElementId, &'static Location<'static>>,
}

impl Children {
    /// Appends an element, rejecting duplicate direct child IDs in debug builds.
    #[track_caller]
    pub fn push(&mut self, parent: &mut impl ParentElement, value: impl IntoElement) {
        self.push_element(parent, value.into_element());
    }

    #[track_caller]
    fn push_element(&mut self, parent: &mut impl ParentElement, element: impl Element) {
        #[cfg(debug_assertions)]
        if let Some(id) = Element::id(&element) {
            let source = Location::caller();

            if let Some(first) = self.ids.insert(id.clone(), source) {
                panic!(
                    "rake::view! duplicate sibling id {id:?}: first at {first}, repeated at {source}; use stable, unique data IDs"
                );
            }
        }

        parent.extend([element.into_any()]);
    }

    /// Appends a child, converting ordinary strings into accessible text nodes.
    #[track_caller]
    pub fn push_child(
        &mut self,
        parent: &mut impl ParentElement,
        value: impl IntoElement,
        id: ElementId,
    ) {
        let element = value.into_element();
        let value = &element as &dyn Any;

        if let Some(text) = value.downcast_ref::<SharedString>() {
            self.push_element(parent, gpui::Text::new(id, text.clone()));
        } else if let Some(text) = value.downcast_ref::<&'static str>() {
            self.push_element(parent, gpui::Text::new(id, SharedString::new_static(text)));
        } else {
            self.push_element(parent, element);
        }
    }
}

/// Mounts an entity owned by the caller, preserving its identity and retained data.
#[doc(hidden)]
pub fn entity<V: gpui::Render>(view: &gpui::Entity<V>) -> gpui::Entity<V> {
    view.clone()
}

/// Configures the tab order of an element, keeping negative indices out of navigation.
#[doc(hidden)]
pub fn tab_index<E: InteractiveElement>(element: E, index: isize) -> E {
    element.tab_index(index).tab_stop(index >= 0)
}

/// Tracks a caller-owned handle, applying an explicit tab index to that same handle.
#[doc(hidden)]
pub fn focus<E: InteractiveElement>(
    element: E,
    focus: &gpui::FocusHandle,
    index: Option<isize>,
) -> E {
    let mut focus = focus.clone();

    if let Some(index) = index {
        focus = focus.tab_index(index).tab_stop(index >= 0);
    }

    element.track_focus(&focus)
}

/// Handles unconsumed Tab/Shift+Tab without installing application-wide key bindings.
#[doc(hidden)]
pub fn tab_navigation(event: &gpui::KeyDownEvent, window: &mut Window, cx: &mut App) {
    let mut modifiers = event.keystroke.modifiers;

    modifiers.shift = false;

    if event.keystroke.key != "tab" || modifiers.modified() || window.default_prevented() {
        return;
    }

    if event.keystroke.modifiers.shift {
        window.focus_prev(cx);
    } else {
        window.focus_next(cx);
    }

    window.prevent_default();
    cx.stop_propagation();
}

/// Constructs a directly rendered image with accessible alternative text.
#[doc(hidden)]
pub fn image(source: impl Into<gpui::ImageSource>, id: impl Into<ElementId>) -> Image {
    Image {
        inner: gpui::img(source).id(id),
        label: SharedString::default(),
        description: None,
    }
}

/// Preserves GPUI image layout, styling, identity and events while exposing its semantics.
#[doc(hidden)]
pub struct Image {
    inner: Stateful<Img>,
    label: SharedString,
    description: Option<SharedString>,
}

impl Image {
    /// Sets alternative text; an empty label marks a decorative image.
    pub fn aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = label.into();

        self
    }

    /// Sets the supplemental accessible description.
    pub fn aria_description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());

        self
    }
}

impl Styled for Image {
    fn style(&mut self) -> &mut StyleRefinement {
        self.inner.style()
    }
}

impl StyledImage for Image {
    fn image_style(&mut self) -> &mut gpui::ImageStyle {
        self.inner.image_style()
    }
}

impl InteractiveElement for Image {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.inner.interactivity()
    }
}

impl StatefulInteractiveElement for Image {}

impl IntoElement for Image {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Image {
    type RequestLayoutState = <Stateful<Img> as Element>::RequestLayoutState;
    type PrepaintState = <Stateful<Img> as Element>::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        Element::id(&self.inner)
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.inner.source_location()
    }

    fn a11y_role(&self) -> Option<accesskit::Role> {
        (!self.label.is_empty()).then_some(accesskit::Role::Image)
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        node.set_label(self.label.to_string());

        if let Some(description) = &self.description {
            node.set_description(description.to_string());
        }
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

#[cfg(test)]
mod tests {
    //! Covers: accessible string conversion and preservation of embedded element identity.
    //! Real: GPUI text and element construction.
    //! Doubles: a collecting ParentElement.
    //! Requires: no display or GPU.

    use super::*;

    #[derive(Default)]
    struct Collected(Vec<gpui::AnyElement>);

    impl ParentElement for Collected {
        fn extend(&mut self, elements: impl IntoIterator<Item = gpui::AnyElement>) {
            self.0.extend(elements);
        }
    }

    #[test]
    fn strings_become_accessible_text_and_custom_elements_keep_their_identity() {
        let mut children = Children::default();
        let mut parent = Collected::default();

        children.push_child(&mut parent, "Literal", "literal".into());
        children.push_child(&mut parent, String::from("Dynamic"), "dynamic".into());

        for (element, (id, value)) in parent
            .0
            .iter_mut()
            .zip([("literal", "Literal"), ("dynamic", "Dynamic")])
        {
            let text = element
                .downcast_mut::<gpui::Text>()
                .expect("accessible text");

            let mut info = accesskit::Node::new(accesskit::Role::Label);

            text.write_a11y_info(&mut info);

            assert_eq!(Element::id(text), Some(id.into()));
            assert_eq!(text.a11y_role(), Some(accesskit::Role::Label));
            assert_eq!(info.value(), Some(value));
        }

        children.push_child(&mut parent, gpui::div().id("custom"), "ignored".into());

        let custom = parent
            .0
            .last_mut()
            .expect("custom child")
            .downcast_mut::<Stateful<gpui::Div>>()
            .expect("original element");

        assert_eq!(Element::id(custom), Some("custom".into()));
    }
}
