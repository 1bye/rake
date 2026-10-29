//! Capture-scoped code rows and ancestry for the supplied inspection boundaries.

use rake::inspection::Element;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn preserve_collapsed(
    old: &BTreeMap<String, String>,
    elements: &[Element],
    collapsed: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut identities: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for element in elements {
        if let Some(stable) = &element.stable_id {
            identities.entry(stable).or_default().push(&element.id);
        }
    }
    collapsed
        .iter()
        .filter_map(|id| old.get(id))
        .filter_map(|stable| identities.get(stable.as_str()))
        .filter(|ids| ids.len() == 1)
        .map(|ids| ids[0].to_owned())
        .collect()
}

pub(crate) const ROW_HEIGHT: f32 = 20.;
pub(crate) const ROW_INSET: f32 = 6.;

pub(crate) fn matches(element: &Element, query: &str) -> bool {
    query.is_empty()
        || format!(
            "{} {} {} {} {}",
            element.display_name(),
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

#[derive(Clone, Copy)]
pub(crate) enum Key {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Toggle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Change {
    Select(usize),
    Expand(usize),
    Collapse(usize),
}

/// Navigate visible nodes; closing tags are annotations of their opening node.
pub(crate) fn navigate(
    elements: &[Element],
    rows: &[Row],
    selected: Option<&str>,
    key: Key,
    page_lines: usize,
) -> Option<Change> {
    let nodes: Vec<_> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| !row.closing)
        .collect();

    let current = nodes
        .iter()
        .position(|(_, row)| Some(elements[row.index].id.as_str()) == selected);

    let last = nodes.len().checked_sub(1)?;

    let Some(position) = current else {
        let position = if matches!(key, Key::Up | Key::End | Key::PageUp) {
            last
        } else {
            0
        };

        return Some(Change::Select(nodes[position].1.index));
    };

    let (line, row) = nodes[position];

    let next = match key {
        Key::Home => 0,
        Key::End => last,
        Key::Up => position.saturating_sub(1),
        Key::Down => (position + 1).min(last),
        Key::PageUp => {
            let line = line.saturating_sub(page_lines.max(1));

            nodes
                .iter()
                .position(|(index, _)| *index >= line)
                .unwrap_or(0)
                .min(position.saturating_sub(1))
        }
        Key::PageDown => {
            let line = line.saturating_add(page_lines.max(1));

            nodes
                .iter()
                .rposition(|(index, _)| *index <= line)
                .unwrap_or(last)
                .max((position + 1).min(last))
        }
        Key::Left if row.branch && !row.collapsed => return Some(Change::Collapse(row.index)),
        Key::Left => nodes.iter().rposition(|(_, parent)| {
            Some(&elements[parent.index].id) == elements[row.index].parent.as_ref()
        })?,
        Key::Right if row.branch && row.collapsed => return Some(Change::Expand(row.index)),
        Key::Right if row.branch => {
            let child = nodes.get(position + 1)?;

            if child.1.depth <= row.depth {
                return None;
            }

            position + 1
        }
        Key::Toggle if row.branch => {
            return Some(if row.collapsed {
                Change::Expand(row.index)
            } else {
                Change::Collapse(row.index)
            });
        }
        Key::Right | Key::Toggle => return None,
    };

    (next != position).then_some(Change::Select(nodes[next].1.index))
}

/// Minimal vertical movement. The caller retains its independent horizontal offset.
pub(crate) fn reveal_offset(current: f32, viewport: f32, line: usize, row_count: usize) -> f32 {
    if viewport <= 0. {
        return current;
    }

    let top = ROW_INSET + line as f32 * ROW_HEIGHT;
    let bottom = top + ROW_HEIGHT;

    let offset = if top < -current {
        -top
    } else if bottom > viewport - current {
        viewport - bottom
    } else {
        current
    };

    let minimum = (viewport - (row_count as f32 * ROW_HEIGHT + 2. * ROW_INSET)).min(0.);

    offset.clamp(minimum, 0.)
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
            view: None,
            layout: None,
            content_mask: None,
            box_model: None,
            styles: None,
            resolved: None,
            stable_id: None,
            navigation: Default::default(),
            metadata_truncated: false,
            paint_order: None,
            style_editable: false,
            style_overrides: Vec::new(),
            accessibility: None,
            handlers: Vec::new(),
            tab_id: None,
            entry_id: None,
        }
    }

    #[test]
    fn collapsed_branches_follow_stable_identities_and_expire_on_removal() {
        let old = BTreeMap::from([
            ("old-root".into(), "root".into()),
            ("old-gone".into(), "gone".into()),
        ]);
        let mut root = element("new-root", None);
        root.stable_id = Some("root".into());
        let collapsed = BTreeSet::from(["old-root".into(), "old-gone".into()]);
        assert_eq!(
            preserve_collapsed(&old, std::slice::from_ref(&root), &collapsed),
            BTreeSet::from(["new-root".into()])
        );
        let mut duplicate = root.clone();
        duplicate.id = "ambiguous".into();
        assert!(preserve_collapsed(&old, &[root, duplicate], &collapsed).is_empty());
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
        assert_eq!(
            rake::inspection::display_name("gpui::Stateful<gpui::Div>"),
            "Div"
        );
    }

    #[test]
    fn arrows_skip_closing_tags_and_stop_at_the_first_and_last_visible_nodes() {
        let elements = vec![
            element("root", None),
            element("branch", Some("root")),
            element("leaf", Some("branch")),
            element("sibling", Some("root")),
        ];

        let visible = rows(&elements, "", &BTreeSet::new());

        assert_eq!(
            navigate(&elements, &visible, Some("leaf"), Key::Down, 3),
            Some(Change::Select(3))
        );
        assert_eq!(
            navigate(&elements, &visible, Some("sibling"), Key::Up, 3),
            Some(Change::Select(2))
        );
        assert_eq!(
            navigate(&elements, &visible, Some("leaf"), Key::Home, 3),
            Some(Change::Select(0))
        );
        assert_eq!(
            navigate(&elements, &visible, Some("root"), Key::End, 3),
            Some(Change::Select(3))
        );
        assert_eq!(
            navigate(&elements, &visible, Some("root"), Key::Up, 3),
            None
        );
        assert_eq!(
            navigate(&elements, &visible, Some("sibling"), Key::Down, 3),
            None
        );
        assert_eq!(
            navigate(&elements, &visible, Some("old-capture-id"), Key::Down, 3),
            Some(Change::Select(0))
        );
        assert_eq!(navigate(&elements, &[], None, Key::Down, 3), None);
    }

    #[test]
    fn horizontal_keys_expand_collapse_and_follow_the_visible_parent_or_child() {
        let elements = vec![
            element("root", None),
            element("branch", Some("root")),
            element("leaf", Some("branch")),
        ];

        let expanded = rows(&elements, "", &BTreeSet::new());
        let collapsed = rows(&elements, "", &BTreeSet::from(["branch".into()]));

        assert_eq!(
            navigate(&elements, &collapsed, Some("branch"), Key::Right, 3),
            Some(Change::Expand(1))
        );
        assert_eq!(
            navigate(&elements, &expanded, Some("branch"), Key::Right, 3),
            Some(Change::Select(2))
        );
        assert_eq!(
            navigate(&elements, &expanded, Some("branch"), Key::Left, 3),
            Some(Change::Collapse(1))
        );
        assert_eq!(
            navigate(&elements, &collapsed, Some("branch"), Key::Left, 3),
            Some(Change::Select(0))
        );
        assert_eq!(
            navigate(&elements, &expanded, Some("leaf"), Key::Left, 3),
            Some(Change::Select(1))
        );
        assert_eq!(
            navigate(&elements, &expanded, Some("leaf"), Key::Right, 3),
            None
        );
        assert_eq!(
            navigate(&elements, &collapsed, Some("branch"), Key::Toggle, 3),
            Some(Change::Expand(1))
        );
        assert_eq!(
            navigate(&elements, &expanded, Some("branch"), Key::Toggle, 3),
            Some(Change::Collapse(1))
        );
    }

    #[test]
    fn page_keys_use_visual_lines_and_filtered_navigation_never_selects_a_hidden_sibling() {
        let elements = vec![
            element("root", None),
            element("branch", Some("root")),
            element("leaf", Some("branch")),
            element("sibling", Some("root")),
        ];

        let visible = rows(&elements, "", &BTreeSet::new());

        assert_eq!(
            navigate(&elements, &visible, Some("root"), Key::PageDown, 3),
            Some(Change::Select(2))
        );
        assert_eq!(
            navigate(&elements, &visible, Some("sibling"), Key::PageUp, 3),
            Some(Change::Select(1))
        );
        let found = rows(&elements, "leaf", &BTreeSet::new());

        assert_eq!(
            navigate(&elements, &found, Some("root"), Key::End, 3),
            Some(Change::Select(2))
        );
        assert_eq!(
            navigate(&elements, &found, Some("leaf"), Key::Down, 3),
            None
        );
        assert!(ancestors(&elements, "old-capture-id").is_empty());
    }

    #[test]
    fn reveal_moves_only_as_far_as_needed_and_clamps_when_content_shrinks() {
        assert_eq!(reveal_offset(-100., 80., 6, 20), -100.);
        assert_eq!(reveal_offset(-100., 80., 0, 20), -6.);
        assert_eq!(reveal_offset(-100., 80., 14, 20), -226.);
        assert_eq!(reveal_offset(-900., 100., 1, 2), 0.);
        assert_eq!(reveal_offset(-100., 0., 6, 20), -100.);
    }
}
