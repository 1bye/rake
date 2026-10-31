//! Copies owned capture metadata; no application evaluation or source-file access.

use super::{Element, MAX_SNAPSHOT_BYTES};
use std::collections::{BTreeMap, BTreeSet};

/// Retained subtree in capture order. Missing targets, duplicate IDs and cycles return `None`.
pub fn subtree<'a>(elements: &'a [Element], id: &str) -> Option<Vec<&'a Element>> {
    Some(
        walk(elements, id)?
            .into_iter()
            .filter_map(|(node, closing)| (!closing).then_some(node))
            .collect(),
    )
}

fn walk<'a>(elements: &'a [Element], id: &str) -> Option<Vec<(&'a Element, bool)>> {
    let index: BTreeMap<_, _> = elements.iter().map(|e| (e.id.as_str(), e)).collect();

    if index.len() != elements.len() {
        return None;
    }

    let mut children: BTreeMap<&str, Vec<&Element>> = BTreeMap::new();

    for element in elements {
        if let Some(parent) = &element.parent {
            children.entry(parent).or_default().push(element);
        }
    }

    let mut stack = vec![(*index.get(id)?, false)];
    let mut seen = BTreeSet::new();
    let mut rows = Vec::new();

    while let Some((node, closing)) = stack.pop() {
        if closing {
            rows.push((node, true));
            continue;
        }

        if !seen.insert(&node.id) {
            return None;
        }

        rows.push((node, false));
        stack.push((node, true));

        if let Some(children) = children.get(node.id.as_str()) {
            stack.extend(children.iter().rev().map(|child| (*child, false)));
        }
    }

    Some(rows)
}

/// Concatenated retained text, including hidden descendants, like DOM textContent.
/// Returns `None` for invalid trees or output above the inspection response budget.
pub fn text(elements: &[Element], id: &str) -> Option<String> {
    let mut output = String::new();

    for node in subtree(elements, id)? {
        if let Some(text) = &node.text {
            output.push_str(text);
        }

        if output.len() > MAX_SNAPSHOT_BYTES {
            return None;
        }
    }

    Some(output)
}

/// Escaped synthetic markup, not the Rust source or a browser DOM serialization.
/// Returns `None` for invalid trees or output above the inspection response budget.
pub fn markup(elements: &[Element], id: &str) -> Option<String> {
    let mut output = String::new();

    for (node, closing) in walk(elements, id)? {
        if let Some(text) = &node.text {
            if !closing {
                output.push_str(&escape(text));
            }
        } else if closing {
            output.push_str(&format!("</{}>", escape(node.display_name())));
        } else {
            output.push_str(&format!("<{}", escape(node.display_name())));

            if let Some(id) = &node.semantic_id {
                output.push_str(&format!(" id=\"{}\"", escape(id)));
            }

            output.push('>');
        }

        if output.len() > MAX_SNAPSHOT_BYTES - 4096 {
            return None;
        }
    }

    (serde_json::to_vec(&output).ok()?.len() < MAX_SNAPSHOT_BYTES - 4096).then_some(output)
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Root-relative inspection path with sibling ordinals and JSON-quoted semantic IDs.
/// This describes the captured hierarchy, not an executable CSS/XPath selector.
/// Missing targets, duplicate IDs, cycles and oversized paths return `None`.
pub fn path(elements: &[Element], id: &str) -> Option<String> {
    let index: BTreeMap<_, _> = elements.iter().map(|e| (e.id.as_str(), e)).collect();

    if index.len() != elements.len() {
        return None;
    }

    let mut counts = BTreeMap::new();
    let mut positions = BTreeMap::new();

    for element in elements {
        let count = counts
            .entry((element.parent.as_deref(), element.display_name()))
            .or_insert(0usize);

        *count += 1;
        positions.insert(element.id.as_str(), *count);
    }

    let mut current = Some(*index.get(id)?);
    let mut seen = BTreeSet::new();
    let mut segments = Vec::new();
    let mut bytes = 0;

    while let Some(node) = current {
        if !seen.insert(&node.id) {
            return None;
        }

        let semantic = node
            .semantic_id
            .as_ref()
            .map(|id| format!("[@id={}]", serde_json::to_string(id).unwrap_or_default()))
            .unwrap_or_default();

        let segment = format!(
            "{}[{}]{semantic}",
            node.display_name(),
            positions[node.id.as_str()]
        );

        bytes += segment.len() + 1;

        if bytes > MAX_SNAPSHOT_BYTES {
            return None;
        }

        segments.push(segment);
        current = match node.parent.as_deref() {
            Some(id) => Some(*index.get(id)?),
            None => None,
        };
    }

    segments.reverse();

    Some(format!("/{}", segments.join("/")))
}
