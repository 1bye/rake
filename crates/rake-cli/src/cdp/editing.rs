//! Literal temporary style declarations; no CSS evaluation or source mutation.

use super::wire::Error;
use rake_protocol::inspection::{Element, StyleDeclaration};
use serde_json::{Value, json};

pub(super) fn text(element: &Element) -> String {
    element
        .style_overrides
        .iter()
        .map(|d| {
            let declaration = format!("{}: {};", d.property, d.value);

            if d.enabled {
                declaration
            } else {
                format!("/* {declaration} */")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn style(node: i32, element: &Element) -> Value {
    let text = text(element);

    let properties: Vec<_> = element.style_overrides.iter().enumerate().map(|(line, d)| {
        let declaration = if d.enabled { format!("{}: {};", d.property, d.value) } else { format!("/* {}: {}; */", d.property, d.value) };

        json!({"name":d.property,"value":d.value,"important":false,"implicit":false,"parsedOk":true,
            "disabled":!d.enabled,"text":declaration,"range":{"startLine":line,"startColumn":0,"endLine":line,"endColumn":declaration.encode_utf16().count()}})
    }).collect();

    let lines: Vec<_> = text.split('\n').collect();

    json!({"styleSheetId":format!("rake-edit-{node}"),"cssProperties":properties,"shorthandEntries":[],"cssText":text,
        "range":{"startLine":0,"startColumn":0,"endLine":lines.len()-1,"endColumn":lines.last().map_or(0, |line| line.encode_utf16().count())}})
}

pub(super) fn rule(node: i32, element: &Element) -> Value {
    json!({"rule":{"styleSheetId":format!("rake-edit-{node}"),"selectorList":{"selectors":[{"text":"[DevTools base overrides]"}],"text":"[DevTools base overrides]"},
        "origin":"regular","style":style(node, element)},"matchingSelectors":[0]})
}

pub(super) fn parse(text: &str) -> Result<Vec<StyleDeclaration>, Error> {
    if text.len() > 8192 {
        return Err(Error::new(-32602, "Override text exceeds 8 KiB"));
    }

    let mut declarations = Vec::new();
    let mut rest = text.trim();

    while !rest.is_empty() {
        let (declaration, tail, enabled) = if let Some(comment) = rest.strip_prefix("/*") {
            let (body, tail) = comment
                .split_once("*/")
                .ok_or_else(|| Error::new(-32602, "Unterminated disabled declaration"))?;

            (body.trim().trim_end_matches(';'), tail, false)
        } else {
            let (body, tail) = rest.split_once(';').unwrap_or((rest, ""));

            (body.trim(), tail, true)
        };

        let (property, value) = declaration
            .split_once(':')
            .ok_or_else(|| Error::new(-32602, "Expected property: value"))?;

        if property.trim().is_empty() || value.trim().is_empty() || declarations.len() == 32 {
            return Err(Error::new(
                -32602,
                "Expected at most 32 nonempty declarations",
            ));
        }

        declarations.push(StyleDeclaration {
            property: property.trim().into(),
            value: value.trim().into(),
            enabled,
        });
        rest = tail.trim();
    }

    Ok(declarations)
}

pub(super) fn replace_range(
    current: &str,
    range: &Value,
    replacement: &str,
) -> Result<String, Error> {
    fn offset(text: &str, line: u64, column: u64) -> Option<usize> {
        let mut start = 0;

        for _ in 0..line {
            start += text.get(start..)?.find('\n')? + 1;
        }

        let line = text.get(start..)?.split('\n').next()?;
        let mut units = 0;

        for (byte, character) in line.char_indices() {
            if units == column {
                return Some(start + byte);
            }

            units += character.len_utf16() as u64;
        }

        (units == column).then_some(start + line.len())
    }
    let start = range["startLine"]
        .as_u64()
        .zip(range["startColumn"].as_u64())
        .and_then(|(l, c)| offset(current, l, c));

    let end = range["endLine"]
        .as_u64()
        .zip(range["endColumn"].as_u64())
        .and_then(|(l, c)| offset(current, l, c));

    let (Some(start), Some(end)) = (start, end) else {
        return Err(Error::new(-32602, "Invalid UTF-16 style range"));
    };

    if start > end {
        return Err(Error::new(-32602, "Reversed style range"));
    }

    Ok(format!(
        "{}{}{}",
        &current[..start],
        replacement,
        &current[end..]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_disabled_values_and_applies_utf16_ranges() {
        let declarations = parse("color: #ffffff;\n/* width: 20px; */").unwrap();

        assert!(declarations[0].enabled);
        assert!(!declarations[1].enabled);
        assert!(parse("/* unclosed").is_err());
        assert_eq!(
            replace_range(
                "a😀b",
                &json!({"startLine":0,"startColumn":1,"endLine":0,"endColumn":3}),
                "x"
            )
            .unwrap(),
            "axb"
        );
        assert!(
            replace_range(
                "😀",
                &json!({"startLine":0,"startColumn":1,"endLine":0,"endColumn":2}),
                "x"
            )
            .is_err()
        );
    }
}
