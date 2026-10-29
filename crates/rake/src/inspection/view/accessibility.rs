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
) -> (Accessibility, Option<String>) {
    let role = inner.a11y_role();
    let mut node = accesskit::Node::new(role.unwrap_or(accesskit::Role::Unknown));
    inner.write_a11y_info(&mut node);
    let any = inner as &dyn std::any::Any;
    let text = if let Some(text) = any.downcast_ref::<gpui::SharedString>() {
        Some(limited(text, 16 * 1024, truncated))
    } else if let Some(text) = any.downcast_ref::<&'static str>() {
        Some(limited(text, 16 * 1024, truncated))
    } else if let Some(text) = any.downcast_ref::<gpui::StyledText>() {
        Some(limited(&text.layout().text(), 16 * 1024, truncated))
    } else if node.role() == accesskit::Role::Label || any.is::<gpui::Text>() {
        node.value().map(|text| limited(text, 16 * 1024, truncated))
    } else {
        None
    };
    let role = (node.role() != accesskit::Role::Unknown).then(|| format!("{:?}", node.role()));
    let label = node.label().map(|label| limited(label, 256, truncated));
    let (name, name_source) = if let Some(label) = &label {
        (Some(label.clone()), Some("label".into()))
    } else if node.role() == accesskit::Role::Label {
        (
            text.as_ref().map(|text| limited(text, 256, truncated)),
            text.as_ref().map(|_| "text".into()),
        )
    } else {
        (None, None)
    };
    let mut states = serde_json::Map::new();
    for (name, value) in [
        ("expanded", node.is_expanded()),
        ("selected", node.is_selected()),
    ] {
        if let Some(value) = value {
            states.insert(name.into(), value.into());
        }
    }
    for (name, value) in [
        ("required", node.is_required()),
        ("readonly", node.is_read_only()),
        ("busy", node.is_busy()),
        ("multiselectable", node.is_multiselectable()),
    ] {
        states.insert(name.into(), value.into());
    }
    if let Some(value) = node.toggled() {
        states.insert(
            if node.role() == accesskit::Role::Button {
                "pressed"
            } else {
                "checked"
            }
            .into(),
            format!("{value:?}").to_lowercase().into(),
        );
    }
    if let Some(value) = node.invalid() {
        states.insert("invalid".into(), format!("{value:?}").to_lowercase().into());
    }
    let focus = focus.and_then(|handle| handle.downgrade().upgrade());
    let accessibility = Accessibility {
        role,
        label,
        name,
        name_source,
        states,
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
    };
    (accessibility, text)
}

pub(super) fn names(snapshot: &mut Snapshot) {
    let indices: std::collections::BTreeMap<_, _> = snapshot
        .elements
        .iter()
        .enumerate()
        .map(|(i, element)| (element.id.as_str(), i))
        .collect();
    let mut children = vec![Vec::new(); snapshot.elements.len()];
    for (i, element) in snapshot.elements.iter().enumerate() {
        if let Some(parent) = element.parent.as_deref().and_then(|id| indices.get(id)) {
            children[*parent].push(i);
        }
    }
    let mut hidden = vec![false; snapshot.elements.len()];
    for i in 0..snapshot.elements.len() {
        hidden[i] |= snapshot.elements[i]
            .accessibility
            .as_ref()
            .is_some_and(|a| a.hidden);
        if hidden[i] {
            if let Some(a11y) = &mut snapshot.elements[i].accessibility {
                a11y.hidden = true;
            }
            for &child in &children[i] {
                hidden[child] = true;
            }
        }
    }
    let mut contents = vec![String::new(); snapshot.elements.len()];
    let mut clipped = vec![false; snapshot.elements.len()];
    for i in (0..snapshot.elements.len()).rev() {
        let element = &snapshot.elements[i];
        if element.accessibility.as_ref().is_some_and(|a| a.hidden) {
            continue;
        }
        if let Some(label) = element
            .accessibility
            .as_ref()
            .and_then(|a| a.label.as_ref())
            .or(element.text.as_ref())
        {
            contents[i] = limited(
                &label.split_whitespace().collect::<Vec<_>>().join(" "),
                256,
                &mut clipped[i],
            );
        } else {
            for &child in &children[i] {
                if contents[child].is_empty() {
                    continue;
                }
                let joined = format!(
                    "{}{}{}",
                    contents[i],
                    if contents[i].is_empty() { "" } else { " " },
                    contents[child]
                );
                contents[i] = limited(&joined, 256, &mut clipped[i]);
                clipped[i] |= clipped[child];
            }
        }
        let element = &mut snapshot.elements[i];
        if let Some(a11y) = &mut element.accessibility
            && a11y.label.is_none()
            && element.text.is_none()
            && matches!(
                a11y.role.as_deref(),
                Some(
                    "Button"
                        | "Link"
                        | "Heading"
                        | "CheckBox"
                        | "RadioButton"
                        | "Switch"
                        | "MenuItem"
                        | "Tab"
                )
            )
        {
            a11y.name = (!contents[i].is_empty()).then(|| contents[i].clone());
            a11y.name_source = a11y.name.as_ref().map(|_| "contents".into());
            element.metadata_truncated |= clipped[i];
        }
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
