//! Read-only CSS presentation of captured base styles, without inventing a cascade.

use rake::inspection::styles::color;
use serde_json::{Value, json};

pub(super) fn properties(value: Option<&Value>) -> Vec<Value> {
    let mut properties = Vec::new();

    let Some(fields) = value.and_then(Value::as_object) else {
        return properties;
    };

    for (key, value) in fields {
        if let Some(fields) = value.as_object().filter(|_| {
            matches!(
                key.as_str(),
                "size"
                    | "min_size"
                    | "max_size"
                    | "padding"
                    | "margin"
                    | "gap"
                    | "inset"
                    | "border_widths"
                    | "corner_radii"
                    | "text"
                    | "overflow"
            )
        }) {
            for (field, value) in fields {
                let field = field.replace('_', "-");

                let name = match key.as_str() {
                    "size" => field,
                    "min_size" => format!("min-{field}"),
                    "max_size" => format!("max-{field}"),
                    "inset" => field,
                    "gap" => format!("{}-gap", if field == "width" { "column" } else { "row" }),
                    "border_widths" => format!("border-{field}-width"),
                    "corner_radii" => format!("border-{field}-radius"),
                    "text" => match field.as_str() {
                        "color" | "font-family" | "font-size" | "font-weight" | "font-style"
                        | "line-height" | "white-space" | "letter-spacing" => field,
                        "align" => "text-align".into(),
                        "text-align" | "text-overflow" | "text-transform" => field,
                        _ => format!("--gpui-text-{field}"),
                    },
                    _ => format!("{}-{field}", key.replace('_', "-")),
                };

                push(&mut properties, &name, value);
            }

            continue;
        }

        let name = match key.as_str() {
            "background" => "background-color".into(),
            "mouse_cursor" => "cursor".into(),
            "display" | "visibility" | "position" | "aspect_ratio" | "flex_direction"
            | "flex_wrap" | "align_items" | "align_self" | "align_content" | "justify_content"
            | "flex_basis" | "flex_grow" | "flex_shrink" | "opacity" | "border_color"
            | "border_style" => key.replace('_', "-"),
            _ => format!("--gpui-{}", key.replace('_', "-")),
        };

        push(&mut properties, &name, value);
    }

    properties
}

fn push(properties: &mut Vec<Value>, name: &str, value: &Value) {
    let css = if value.is_null() && matches!(name, "background-color" | "border-color") {
        Some("transparent".into())
    } else if matches!(name, "color" | "background-color" | "border-color") {
        color(value).map(|color| color.css())
    } else if name == "font-family" && value.is_string() {
        Some(value.to_string())
    } else if matches!(name, "max-width" | "max-height") && value == "auto" {
        Some("none".into())
    } else if !name.starts_with("--gpui-") {
        match value {
            Value::String(value) => Some(
                if matches!(
                    name,
                    "display"
                        | "visibility"
                        | "position"
                        | "flex-direction"
                        | "flex-wrap"
                        | "align-items"
                        | "align-self"
                        | "align-content"
                        | "justify-content"
                        | "overflow-x"
                        | "overflow-y"
                        | "border-style"
                        | "font-style"
                        | "white-space"
                        | "text-align"
                        | "text-transform"
                        | "cursor"
                ) {
                    if value == "NoWrap" {
                        "nowrap".into()
                    } else if name == "cursor" && value == "Arrow" {
                        "default".into()
                    } else if name == "cursor" && value == "PointingHand" {
                        "pointer".into()
                    } else if name == "cursor" && value == "IBeam" {
                        "text".into()
                    } else {
                        kebab(value)
                    }
                } else {
                    value.clone()
                },
            ),
            Value::Number(_) => Some(value.to_string()),
            _ => None,
        }
    } else {
        None
    };

    let (name, text) = if let Some(css) = css {
        (name.to_owned(), css)
    } else {
        (
            if name.starts_with("--gpui-") {
                name.to_owned()
            } else {
                format!("--gpui-{name}")
            },
            value.to_string(),
        )
    };

    properties
        .push(json!({"name":name,"value":text,"implicit":false,"parsedOk":true,"disabled":false}));
}

fn kebab(value: &str) -> String {
    let mut result = String::new();

    for (index, character) in value.chars().enumerate() {
        if character.is_ascii_uppercase() && index > 0 {
            result.push('-');
        }

        result.push(character.to_ascii_lowercase());
    }

    result
}

pub(super) fn inline(value: Option<&Value>) -> Value {
    let properties = properties(value);

    let text = properties
        .iter()
        .map(|p| {
            format!(
                "{}: {};",
                p["name"].as_str().unwrap(),
                p["value"].as_str().unwrap()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    json!({"cssProperties":properties,"shorthandEntries":[],"cssText":text})
}

#[cfg(test)]
mod tests {
    // Real captured GPUI color shapes and authored units; no renderer or CSS engine.
    use super::*;

    #[test]
    fn solid_colors_typography_edges_and_unknown_fields_remain_visible() {
        let styles = json!({
            "background":{"Color":{"tag":"Solid","color_space":"Srgb","solid":{"h":0.5441176295280457,"s":0.2931034564971924,"l":0.22745099663734436,"a":1.0}}},
            "text":{"color":{"hue":79.99999237060547,"saturation":0.3529411852359772,"lightness":0.9333333373069764,"alpha":1.0},"font_size":"1.25rem","font_weight":600},
            "padding":{"top":"0.5rem"},"border_widths":{"left":"2px"},"corner_radii":{"top_left":"4px"},
            "overflow":{"x":"Hidden"},"mouse_cursor":"PointingHand","future":{"keep":true}
        });

        let properties = properties(Some(&styles));

        let get = |name| {
            properties.iter().find(|p| p["name"] == name).unwrap()["value"]
                .as_str()
                .unwrap()
        };

        assert_eq!(get("background-color"), "#29424b");
        assert_eq!(get("color"), "#f0f4e8");
        assert_eq!(get("font-size"), "1.25rem");
        assert_eq!(get("font-weight"), "600");
        assert_eq!(get("padding-top"), "0.5rem");
        assert_eq!(get("border-left-width"), "2px");
        assert_eq!(get("border-top-left-radius"), "4px");
        assert_eq!(get("overflow-x"), "hidden");
        assert_eq!(get("cursor"), "pointer");
        assert_eq!(get("--gpui-future"), "{\"keep\":true}");
        assert!(!properties.iter().any(|p| p["name"] == "padding-right"));
    }

    #[test]
    fn unknown_color_spaces_and_non_solid_fills_are_preserved_as_metadata() {
        for tag in ["Gradient", "Solid"] {
            let value = json!({"background":{"Color":{"tag":tag,"color_space":"Oklab","solid":{"h":0.5,"s":1.0,"l":0.5,"a":1.0}}}});
            let properties = properties(Some(&value));

            assert_eq!(properties[0]["name"], "--gpui-background-color");
            assert!(properties[0]["value"].as_str().unwrap().contains("Oklab"));
        }
    }

    #[test]
    fn resolved_defaults_use_css_spellings() {
        let properties = properties(Some(&json!({
            "flex_wrap":"NoWrap", "mouse_cursor":"Arrow", "background":null,
            "max_size":{"width":"auto"}, "text":{"font_family":".SystemUIFont","white_space":"NoWrap"}
        })));

        let get = |name| {
            properties.iter().find(|p| p["name"] == name).unwrap()["value"]
                .as_str()
                .unwrap()
        };

        assert_eq!(get("flex-wrap"), "nowrap");
        assert_eq!(get("white-space"), "nowrap");
        assert_eq!(get("font-family"), "\".SystemUIFont\"");
        assert_eq!(get("max-width"), "none");
        assert_eq!(get("background-color"), "transparent");
        assert_eq!(get("cursor"), "default");
    }
}
