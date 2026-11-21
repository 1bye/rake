//! GPUI-only types and accessibility adapter for the shared component source.
use rake::gpui::{
    self,
    accesskit::{Role, Toggled},
    prelude::*,
};

pub type Element = gpui::AnyElement;
pub type Children = Vec<Element>;
pub type Theme = ();
pub type Handler = Box<dyn Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App)>;
pub struct Signal {
    pub checked: bool,
    pub toggle: Handler,
}
pub fn finish(element: impl IntoElement) -> Element {
    element.into_any_element()
}
pub fn slot(children: Children) -> Element {
    gpui::div()
        .flex()
        .flex_col()
        .gap_4()
        .children(children)
        .into_any_element()
}
pub fn switch_semantics(element: Element, checked: bool) -> Element {
    gpui::div()
        .id("notifications-state")
        .role(Role::Switch)
        .aria_toggled(Toggled::from(checked))
        .child(element)
        .into_any_element()
}
