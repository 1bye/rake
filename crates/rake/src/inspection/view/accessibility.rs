//! Read-only GPUI accessibility metadata; no platform tree or synthetic child mutation.

use super::super::{Accessibility, Snapshot};
use super::limited;
use gpui::{App, Element, FocusHandle, Window, accesskit};

pub(super) fn capture<E: Element>(
    inner: &E,
    focus: Option<&FocusHandle>,
    style: Option<&gpui::Style>,
    window: &Window,
    cx: &App,
    truncated: &mut bool,
) -> Accessibility {
    let role = inner.a11y_role();
    let mut node = accesskit::Node::new(role.unwrap_or(accesskit::Role::Unknown));
    inner.write_a11y_info(&mut node);
    let role = (node.role() != accesskit::Role::Unknown).then(|| format!("{:?}", node.role()));
    let focus = focus.and_then(|handle| handle.downgrade().upgrade());
    Accessibility {
        role,
        label: node.label().map(|label| limited(label, 256, truncated)),
        description: node
            .description()
            .map(|label| limited(label, 512, truncated)),
        disabled: node.is_disabled(),
        hidden: node.is_hidden()
            || style.is_some_and(|style| {
                style.visibility == gpui::Visibility::Hidden || style.display == gpui::Display::None
            }),
        focusable: node.supports_action(accesskit::Action::Focus) && !node.is_disabled(),
        focused: focus.as_ref().map(|handle| handle.is_focused(window)),
        contains_focus: focus
            .as_ref()
            .map(|handle| handle.contains_focused(window, cx)),
        tab_index: focus.as_ref().map(|handle| handle.tab_index),
        tab_stop: focus.as_ref().map(|handle| handle.tab_stop),
        captured_tab_order: None,
        actions: (0..32)
            .filter_map(accesskit::Action::n)
            .filter(|action| node.supports_action(*action))
            .map(|action| format!("{action:?}"))
            .collect(),
    }
}

pub(super) fn order(snapshot: &mut Snapshot) {
    let mut stops: Vec<_> = snapshot
        .elements
        .iter()
        .enumerate()
        .filter_map(|(index, element)| {
            let a11y = element.accessibility.as_ref()?;
            (a11y.tab_stop == Some(true)
                && !a11y.disabled
                && !a11y.hidden
                && element.paint_order.is_some())
            .then_some((a11y.tab_index?, element.paint_order.unwrap_or(index), index))
        })
        .collect();
    stops.sort_unstable();
    for (order, (_, _, index)) in stops.into_iter().enumerate() {
        if let Some(a11y) = &mut snapshot.elements[index].accessibility {
            a11y.captured_tab_order = Some(order + 1);
        }
    }
}
