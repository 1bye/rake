//! Observed origins of resolved properties; GPUI's private interaction rules are not inferred.

use serde_json::{Map, Value};

use super::super::{StyleLayer, StyleOrigin};

pub(super) fn capture(
    resolved: &Value,
    authored: &Value,
    base: &Value,
    live: &Value,
    override_refinement: Option<&Value>,
) -> Vec<StyleLayer> {
    let mut layers = [
        StyleOrigin::Default,
        StyleOrigin::Inherited,
        StyleOrigin::Authored,
        StyleOrigin::Override,
        StyleOrigin::Interaction,
        StyleOrigin::Layout,
    ]
    .map(|origin| StyleLayer {
        origin,
        properties: Value::Object(Map::new()),
    });
    let Some(fields) = resolved.as_object() else {
        return Vec::new();
    };
    for (name, value) in fields {
        let grouped = matches!(
            name.as_str(),
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
        );
        let mut insert = |field: Option<&str>, value: &Value| {
            let origin = if name == "size" {
                StyleOrigin::Layout
            } else if at(base, name, field) != at(live, name, field) {
                StyleOrigin::Interaction
            } else if override_refinement.is_some_and(|refinement| {
                !at(refinement, name, field).is_null()
                    && at(refinement, name, field) != at(authored, name, field)
            }) {
                StyleOrigin::Override
            } else if !at(authored, name, field).is_null() {
                StyleOrigin::Authored
            } else if name == "text" && at(live, name, field).is_null() {
                StyleOrigin::Inherited
            } else {
                StyleOrigin::Default
            };
            let properties = layers
                .iter_mut()
                .find(|layer| layer.origin == origin)
                .unwrap()
                .properties
                .as_object_mut()
                .unwrap();
            if let Some(field) = field {
                properties
                    .entry(name.clone())
                    .or_insert_with(|| Value::Object(Map::new()))
                    .as_object_mut()
                    .unwrap()
                    .insert(field.into(), value.clone());
            } else {
                properties.insert(name.clone(), value.clone());
            }
        };
        if let Some(fields) = value.as_object().filter(|_| grouped) {
            for (field, value) in fields {
                insert(Some(field), value);
            }
        } else {
            insert(None, value);
        }
    }
    layers
        .into_iter()
        .filter(|layer| layer.properties.as_object().is_some_and(|p| !p.is_empty()))
        .collect()
}

fn at<'a>(root: &'a Value, name: &str, field: Option<&str>) -> &'a Value {
    field.map_or_else(|| &root[name], |field| &root[name][field])
}

#[cfg(test)]
mod tests {
    // Real provenance partition; synthetic GPUI JSON, no renderer.
    use super::*;
    use serde_json::json;

    #[test]
    fn resolves_origins_per_property_without_attributing_uninspected_ancestors() {
        let authored = json!({"opacity":0.8, "text":{"font_size":"1rem"}});
        let base = json!({"opacity":0.8, "text":{"font_size":"1rem"}, "background":null});
        let live = json!({"opacity":0.8, "text":{"font_size":"1rem"}, "background":"hover"});
        let resolved = json!({"opacity":0.8, "text":{"font_size":"16px", "color":"parent"}, "background":"hover", "display":"Flex", "size":{"width":"80px"}});
        let layers = capture(&resolved, &authored, &base, &live, None);
        let get = |origin| {
            &layers
                .iter()
                .find(|layer| layer.origin == origin)
                .unwrap()
                .properties
        };
        assert_eq!(
            get(StyleOrigin::Inherited),
            &json!({"text":{"color":"parent"}})
        );
        assert_eq!(
            get(StyleOrigin::Authored),
            &json!({"opacity":0.8,"text":{"font_size":"16px"}})
        );
        assert_eq!(
            get(StyleOrigin::Interaction),
            &json!({"background":"hover"})
        );
        assert_eq!(get(StyleOrigin::Default), &json!({"display":"Flex"}));
        assert_eq!(get(StyleOrigin::Layout), &json!({"size":{"width":"80px"}}));
    }
}
