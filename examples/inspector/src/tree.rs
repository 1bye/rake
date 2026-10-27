//! Capture-scoped code rows and ancestry for the supplied inspection boundaries.

use rake::inspection::Element;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn short_kind(kind: &str) -> String {
    kind.split_inclusive(['<', '>', ','])
        .map(|part| part.rsplit("::").next().unwrap_or(part))
        .collect()
}

pub(crate) fn tag(kind: &str) -> String {
    let kind = short_kind(kind);

    kind.strip_prefix("Stateful<")
        .and_then(|kind| kind.strip_suffix('>'))
        .unwrap_or(&kind)
        .to_owned()
}

pub(crate) fn matches(element: &Element, query: &str) -> bool {
    query.is_empty()
        || format!(
            "{} {} {} {}",
            element.id,
            element.kind,
            element.semantic_id.as_deref().unwrap_or(""),
            element
                .source
                .as_ref()
                .map_or("", |source| source.file.as_str())
        )
        .to_lowercase()
        .contains(&query.to_lowercase())
}

pub(crate) fn ancestors<'a>(elements: &'a [Element], id: &str) -> Vec<&'a Element> {
    let index: BTreeMap<_, _> = elements
        .iter()
        .map(|element| (element.id.as_str(), element))
        .collect();

    let mut path = Vec::new();
    let mut next = Some(id);
    let mut visited = BTreeSet::new();

    while let Some(element) = next.and_then(|id| index.get(id)).copied() {
        if !visited.insert(&element.id) {
            break;
        }

        path.push(element);
        next = element.parent.as_deref();
    }

    path.reverse();

    path
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Row {
    pub index: usize,
    pub depth: usize,
    pub closing: bool,
    pub branch: bool,
    pub collapsed: bool,
    pub matched: bool,
}

pub(crate) fn rows(elements: &[Element], query: &str, collapsed: &BTreeSet<String>) -> Vec<Row> {
    let index: BTreeMap<_, _> = elements
        .iter()
        .enumerate()
        .map(|(i, element)| (element.id.as_str(), i))
        .collect();

    let mut retained = vec![query.is_empty(); elements.len()];

    for element in elements.iter().filter(|element| matches(element, query)) {
        for ancestor in ancestors(elements, &element.id) {
            retained[index[ancestor.id.as_str()]] = true;
        }
    }

    let mut children = vec![Vec::new(); elements.len()];
    let mut roots = Vec::new();

    for (i, element) in elements.iter().enumerate() {
        if !retained[i] {
            continue;
        }

        if let Some(parent) = element
            .parent
            .as_deref()
            .and_then(|id| index.get(id))
            .copied()
            .filter(|parent| *parent != i)
        {
            children[parent].push(i);
        } else {
            roots.push(i);
        }
    }

    // A bounded traversal also tolerates malformed cycles in a remote capture.
    let mut result = Vec::new();
    let mut visited = BTreeSet::new();

    for root in roots.into_iter().chain(0..elements.len()) {
        if !retained[root] || visited.contains(&root) {
            continue;
        }

        let mut stack = vec![(root, 0, false)];

        while let Some((i, depth, closing)) = stack.pop() {
            let branch = !children[i].is_empty();
            let collapsed = collapsed.contains(&elements[i].id);

            if closing {
                result.push(Row {
                    index: i,
                    depth,
                    closing: true,
                    branch,
                    collapsed: false,
                    matched: false,
                });
                continue;
            }

            if !visited.insert(i) {
                continue;
            }

            result.push(Row {
                index: i,
                depth,
                closing: false,
                branch,
                collapsed,
                matched: !query.is_empty() && matches(&elements[i], query),
            });

            if branch && !collapsed {
                stack.push((i, depth, true));

                for &child in children[i].iter().rev() {
                    stack.push((child, depth + 1, false));
                }
            } else if collapsed {
                let mut hidden = children[i].clone();

                while let Some(child) = hidden.pop() {
                    if visited.insert(child) {
                        hidden.extend(&children[child]);
                    }
                }
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    //! Covers: matched ancestry, disclosures, closing tags and malformed parent cycles.
    //! Real: tree presentation. Doubles: supplied capture nodes. Requires: no GPU.
    use super::*;

    fn element(id: &str, parent: Option<&str>) -> Element {
        Element {
            id: id.into(),
            parent: parent.map(str::to_owned),
            kind: "gpui::Div".into(),
            semantic_id: Some(id.into()),
            source: None,
            layout: None,
            styles: None,
            metadata_truncated: false,
            tab_id: None,
            entry_id: None,
        }
    }

    #[test]
    fn disclosure_hides_descendants_and_search_reveals_the_exact_ancestor_path() {
        let tree = vec![
            element("workspace", None),
            element("sidebar", Some("workspace")),
            element("editor", Some("workspace")),
            element("body", Some("editor")),
        ];

        let collapsed = BTreeSet::from(["editor".into()]);
        let visible = rows(&tree, "", &collapsed);

        assert_eq!(
            visible
                .iter()
                .map(|row| (row.index, row.closing))
                .collect::<Vec<_>>(),
            [(0, false), (1, false), (2, false), (0, true)]
        );
        assert!(visible[2].branch && visible[2].collapsed);
        let found = rows(&tree, "BODY", &BTreeSet::new());

        assert_eq!(
            found
                .iter()
                .filter(|row| !row.closing)
                .map(|row| row.index)
                .collect::<Vec<_>>(),
            [0, 2, 3]
        );
        assert_eq!(found[2].depth, 2);
        assert!(found[2].matched);
        assert_eq!(
            ancestors(&tree, "body")
                .iter()
                .map(|element| element.id.as_str())
                .collect::<Vec<_>>(),
            ["workspace", "editor", "body"]
        );
    }

    #[test]
    fn cycles_are_bounded_and_rust_wrappers_do_not_become_nested_tag_syntax() {
        let tree = vec![element("a", Some("b")), element("b", Some("a"))];

        assert_eq!(ancestors(&tree, "a").len(), 2);
        assert_eq!(
            rows(&tree, "", &BTreeSet::new())
                .iter()
                .filter(|row| !row.closing)
                .count(),
            2
        );
        assert_eq!(tag("gpui::Stateful<gpui::Div>"), "Div");
    }
}
