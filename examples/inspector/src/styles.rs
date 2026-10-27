//! Read-only presentation of the supplied GPUI refinements; never a CSS cascade.

use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Color {
    pub h: f32,
    pub s: f32,
    pub l: f32,
    pub a: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Group {
    Layout,
    Spacing,
    Appearance,
    Typography,
    Other,
}

impl Group {
    pub fn label(self) -> &'static str {
        match self {
            Self::Layout => "Layout",
            Self::Spacing => "Spacing",
            Self::Appearance => "Appearance",
            Self::Typography => "Typography",
            Self::Other => "Other",
        }
    }
}

pub(crate) struct Declaration<'a> {
    pub name: String,
    pub value: String,
    pub raw: &'a Value,
    pub color: Option<Color>,
    pub group: Group,
}

fn number(value: f64) -> String {
    format!("{value:.3}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

fn color(value: &Value) -> Option<Color> {
    let (value, names, hue_scale) = if value.get("Color").is_some() {
        let color = &value["Color"];

        if color["tag"] != "Solid"
            || color
                .get("color_space")
                .is_some_and(|space| space != "Srgb")
        {
            return None;
        }

        (&color["solid"], ["h", "s", "l", "a"], 1.)
    } else {
        (value, ["hue", "saturation", "lightness", "alpha"], 360.)
    };

    let values: Option<Vec<_>> = names.into_iter().map(|name| value[name].as_f64()).collect();
    let values = values?;

    if !(0.0..=hue_scale).contains(&values[0])
        || values[1..].iter().any(|value| !(0.0..=1.0).contains(value))
    {
        return None;
    }

    Some(Color {
        h: (values[0] / hue_scale) as f32,
        s: values[1] as f32,
        l: values[2] as f32,
        a: values[3] as f32,
    })
}

pub(crate) fn summary(value: &Value) -> String {
    if let Some(color) = color(value) {
        let chroma = (1. - (2. * color.l - 1.).abs()) * color.s;
        let sector = color.h * 6.;
        let second = chroma * (1. - (sector % 2. - 1.).abs());

        let (r, g, b) = match sector as u8 {
            0 | 6 => (chroma, second, 0.),
            1 => (second, chroma, 0.),
            2 => (0., chroma, second),
            3 => (0., second, chroma),
            4 => (second, 0., chroma),
            _ => (chroma, 0., second),
        };

        let offset = color.l - chroma / 2.;
        let channel = |value: f32| ((value + offset) * 255.).round() as u8;
        let mut value = format!("#{:02x}{:02x}{:02x}", channel(r), channel(g), channel(b));

        if color.a < 1. {
            value.push_str(&format!("{:02x}", (color.a * 255.).round() as u8));
        }

        return value;
    }

    match value {
        Value::String(value) => value.clone(),
        Value::Number(value) if value.is_f64() => number(value.as_f64().unwrap_or_default()),
        Value::Object(fields) => {
            let parts = fields
                .iter()
                .take(3)
                .map(|(name, value)| {
                    format!(
                        "{name}: {}",
                        if value.is_object() || value.is_array() {
                            "…".into()
                        } else {
                            summary(value)
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");

            format!("{{ {parts}{} }}", if fields.len() > 3 { ", …" } else { "" })
        }
        _ => value.to_string(),
    }
}

fn group(name: &str) -> Group {
    if name.starts_with("text-") || name.starts_with("font-") {
        Group::Typography
    } else if ["padding", "margin", "inset"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
    {
        Group::Spacing
    } else if ["background", "border", "corner", "opacity", "shadow"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
    {
        Group::Appearance
    } else if [
        "display", "position", "size", "width", "height", "min-", "max-", "flex", "align",
        "justify", "gap", "overflow",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
    {
        Group::Layout
    } else {
        Group::Other
    }
}

fn push<'a>(rows: &mut Vec<Declaration<'a>>, name: String, raw: &'a Value) {
    let mut value = summary(raw);

    if matches!(
        name.as_str(),
        "display"
            | "position"
            | "flex-direction"
            | "flex-wrap"
            | "align-items"
            | "align-self"
            | "justify-content"
            | "overflow-x"
            | "overflow-y"
            | "mouse-cursor"
            | "text-font-style"
    ) && raw.is_string()
    {
        let mut normalized = String::new();

        for (i, c) in value.chars().enumerate() {
            if c.is_ascii_uppercase() && i > 0 {
                normalized.push('-');
            }

            normalized.push(c.to_ascii_lowercase());
        }

        value = normalized;
    }

    rows.push(Declaration {
        group: group(&name),
        name,
        value,
        color: color(raw),
        raw,
    });
}

pub(crate) fn declarations(styles: &Value) -> Vec<Declaration<'_>> {
    let mut rows = Vec::new();

    if let Some(fields) = styles.as_object() {
        for (name, value) in fields {
            if matches!(
                name.as_str(),
                "size"
                    | "min_size"
                    | "max_size"
                    | "margin"
                    | "padding"
                    | "inset"
                    | "border_widths"
                    | "corner_radii"
                    | "text"
            ) && let Some(fields) = value.as_object()
            {
                for (field, value) in fields {
                    let field = field.replace('_', "-");

                    let name = match name.as_str() {
                        "size" => field,
                        "min_size" => format!("min-{field}"),
                        "max_size" => format!("max-{field}"),
                        "border_widths" => format!("border-{field}-width"),
                        "corner_radii" => format!("border-{field}-radius"),
                        _ => format!("{}-{field}", name.replace('_', "-")),
                    };

                    push(&mut rows, name, value);
                }
            } else {
                push(&mut rows, name.replace('_', "-"), value);
            }
        }
    }

    rows.sort_by_key(|row| row.group);

    rows
}

pub(crate) fn matches(row: &Declaration<'_>, query: &str) -> bool {
    query.is_empty()
        || format!("{} {} {}", row.name, row.value, row.raw)
            .to_lowercase()
            .contains(&query.to_lowercase())
}

#[cfg(test)]
mod tests {
    //! Covers: readable authored values, nested dimensions, filtering and honest color swatches.
    //! Real: captured-style JSON projection. Doubles: sample metadata. Requires: no GPU.
    use super::*;
    use serde_json::json;

    #[test]
    fn preserves_units_and_expands_authored_edges_without_inventing_missing_values() {
        let styles = json!({"padding":{"top":"0.75rem", "left":"12px"}, "min_size":{"height":"0px"}, "flex_grow":1.0, "display":"Flex", "future_refinement":{"custom":true}});
        let rows = declarations(&styles);

        assert_eq!(
            rows.iter()
                .find(|row| row.name == "padding-top")
                .unwrap()
                .value,
            "0.75rem"
        );
        assert!(
            rows.iter()
                .any(|row| row.name == "min-height" && row.value == "0px")
        );
        assert!(
            rows.iter()
                .any(|row| row.name == "flex-grow" && row.value == "1")
        );
        assert!(!rows.iter().any(|row| row.name == "padding-right"));
        assert!(
            rows.iter()
                .any(|row| row.name == "display" && row.value == "flex")
        );
        assert!(matches(
            rows.iter().find(|row| row.name == "padding-left").unwrap(),
            "12PX"
        ));
        assert!(
            rows.iter()
                .any(|row| row.name == "future-refinement" && row.raw["custom"] == true)
        );
    }

    #[test]
    fn colors_have_swatches_only_for_recognized_valid_solid_values() {
        let styles = json!({"background":{"Color":{"tag":"Solid", "solid":{"h":0.5,"s":1.0,"l":0.5,"a":0.75}}}, "text":{"color":{"hue":180.0,"saturation":1.0,"lightness":0.5,"alpha":0.75}}});
        let rows = declarations(&styles);

        assert!(
            rows.iter()
                .all(|row| row.color.is_some() && row.value == "#00ffffbf")
        );
        assert!(
            color(&json!({"Color":{"tag":"Gradient","solid":{"h":0.5,"s":1.0,"l":0.5,"a":1.0}}}))
                .is_none()
        );
        assert!(color(&json!({"hue":999,"saturation":1,"lightness":0.5,"alpha":1})).is_none());
    }
}
