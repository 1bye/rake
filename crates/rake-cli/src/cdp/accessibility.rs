//! Accessibility projection of inspected GPUI boundaries, not a browser/platform AX tree.

use rake_protocol::inspection::Element;
use serde_json::{Value, json};

pub(super) fn node(
    id: i32,
    element: Option<&Element>,
    parent: Option<i32>,
    children: Vec<i32>,
) -> Value {
    let a11y = element.and_then(|element| element.accessibility.as_ref());
    let ignored = id != 1 && a11y.is_none_or(|a11y| a11y.role.is_none() || a11y.hidden);

    let role = if id == 1 {
        "application"
    } else {
        a11y.and_then(|a11y| a11y.role.as_deref())
            .map_or("generic", |role| match role {
                "Button" => "button",
                "Link" => "link",
                "Heading" => "heading",
                "CheckBox" => "checkbox",
                "RadioButton" => "radio",
                "TextInput" | "MultilineTextInput" => "textbox",
                "List" => "list",
                "ListItem" => "listitem",
                "Paragraph" => "paragraph",
                "Label" => "StaticText",
                "Switch" => "switch",
                "Tab" => "tab",
                "MenuItem" => "menuitem",
                "Image" => "image",
                "GenericContainer" => "generic",
                other => other,
            })
    };

    let mut result = json!({"nodeId":format!("ax-{id}"),"backendDOMNodeId":id,"ignored":ignored,
        "role":{"type":"role","value":role},"childIds":children.into_iter().map(|id|format!("ax-{id}")).collect::<Vec<_>>()});

    if let Some(parent) = parent {
        result["parentId"] = json!(format!("ax-{parent}"));
    }

    if let Some(a11y) = a11y {
        let mut properties = vec![
            json!({"name":"focusable","value":{"type":"boolean","value":a11y.focusable}}),
            json!({"name":"disabled","value":{"type":"boolean","value":a11y.disabled}}),
        ];

        if let Some(focused) = a11y.focused {
            properties.push(json!({"name":"focused","value":{"type":"boolean","value":focused}}));
        }

        for (name, value) in &a11y.states {
            let kind = if matches!(name.as_str(), "checked" | "pressed") {
                "tristate"
            } else if value.is_boolean() {
                "boolean"
            } else {
                "token"
            };

            properties.push(json!({"name":name,"value":{"type":kind,"value":value}}));
        }

        result["properties"] = json!(properties);

        if let Some(label) = a11y.name.as_ref().or(a11y.label.as_ref()) {
            result["name"] = json!({"type":"string","value":label});
        }

        if let Some(description) = &a11y.description {
            result["description"] = json!({"type":"string","value":description});
        }

        if ignored {
            result["ignoredReasons"] = json!([{"name":if a11y.hidden { "notRendered" } else { "uninteresting" },"value":{"type":"boolean","value":true}}]);
        }
    }

    result
}
