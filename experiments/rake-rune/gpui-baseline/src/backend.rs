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
pub fn finish_button(
    element: impl IntoElement + StatefulInteractiveElement + Styled,
    checked: Option<bool>,
) -> Element {
    let palette = *rake::theme::current::<rake::theme::Semantic>();
    let element = element
        .border_1()
        .border_color(palette.muted)
        .hover(move |style| style.bg(palette.accent).border_color(palette.ring))
        .focus_visible(move |style| style.bg(palette.accent).border_color(palette.ring));
    button_state(element, checked).into_any_element()
}

fn button_state<E: StatefulInteractiveElement>(element: E, checked: Option<bool>) -> E {
    match checked {
        Some(checked) => element
            .role(Role::Switch)
            .aria_toggled(Toggled::from(checked)),
        None => element,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_state_preserves_the_button_identity_without_a_wrapper() {
        for checked in [None, Some(false), Some(true)] {
            let button = gpui::div().id("notifications").role(Role::Button);
            let id = gpui::Element::id(&button);
            let button = button_state(button, checked);
            let mut node = gpui::accesskit::Node::new(Role::Button);
            gpui::Element::write_a11y_info(&button, &mut node);

            assert_eq!(gpui::Element::id(&button), id);
            assert_eq!(
                gpui::Element::a11y_role(&button),
                Some(if checked.is_some() {
                    Role::Switch
                } else {
                    Role::Button
                })
            );
            assert_eq!(node.toggled(), checked.map(Toggled::from));
        }
    }
}
