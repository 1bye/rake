//! Linear captured-text storage: subtree ranges share one buffer instead of copying ancestors.

use crate::cdp::wire::Error;
use rake_protocol::inspection::Snapshot;
use std::{collections::BTreeMap, ops::Range};

pub(super) struct Content {
    text: String,
    ranges: Vec<Range<usize>>,
    children: Vec<Vec<usize>>,
    complete: Vec<bool>,
}

impl Content {
    pub fn new(tree: &Snapshot) -> Result<Self, Error> {
        let n = tree.elements.len();
        let ids: BTreeMap<_, _> = tree
            .elements
            .iter()
            .enumerate()
            .map(|(i, e)| (e.id.as_str(), i))
            .collect();
        if ids.len() != n {
            return Err(Error::new(-32000, "Duplicate captured node identities"));
        }
        let bytes = tree
            .elements
            .iter()
            .try_fold(0usize, |sum, e| {
                sum.checked_add(e.text.as_ref().map_or(0, String::len))
            })
            .filter(|n| *n <= 16 * 1024 * 1024)
            .ok_or_else(|| Error::new(-32000, "Captured text exceeds 16 MiB"))?;
        let mut result = Self {
            text: String::with_capacity(bytes),
            ranges: vec![0..0; n],
            children: vec![Vec::new(); n],
            complete: vec![tree.omitted == 0; n],
        };
        let mut roots = Vec::new();
        for (i, e) in tree.elements.iter().enumerate() {
            if let Some(parent) = e.parent.as_deref().and_then(|id| ids.get(id)) {
                result.children[*parent].push(i);
            } else {
                roots.push(i);
            }
        }
        roots.reverse();
        let mut seen = vec![false; n];
        // A boundary's own rendered text takes precedence. Its children get separate
        // ranges later, outside the enclosing ancestor's canonical text interval.
        while let Some(root) = roots.pop() {
            let mut work = vec![(root, false)];
            while let Some((i, end)) = work.pop() {
                if end {
                    result.ranges[i].end = result.text.len();
                    result.complete[i] &= result.children[i]
                        .iter()
                        .all(|child| result.complete[*child]);
                    continue;
                }
                if seen[i] {
                    return Err(Error::new(-32000, "Cyclic captured parent chain"));
                }
                seen[i] = true;
                result.ranges[i].start = result.text.len();
                result.complete[i] &= !tree.elements[i].metadata_truncated;
                if let Some(text) = &tree.elements[i].text {
                    result.text.push_str(text);
                    result.ranges[i].end = result.text.len();
                    roots.extend(result.children[i].iter().rev());
                } else {
                    work.push((i, true));
                    work.extend(result.children[i].iter().rev().map(|child| (*child, false)));
                }
            }
        }
        if seen.iter().any(|visited| !*visited) {
            return Err(Error::new(-32000, "Unreachable captured parent cycle"));
        }
        Ok(result)
    }
    pub fn text(&self, i: usize) -> &str {
        &self.text[self.ranges[i].clone()]
    }
    pub fn complete(&self, i: usize) -> bool {
        self.complete[i]
    }
    pub fn scope(&self, root: usize) -> Vec<bool> {
        let mut included = vec![false; self.ranges.len()];
        let mut work = vec![root];
        while let Some(i) = work.pop() {
            if included[i] {
                continue;
            }
            included[i] = true;
            work.extend(&self.children[i]);
        }
        included
    }
    pub fn contains(&self, needle: &str) -> Vec<bool> {
        let mut matches = vec![false; self.ranges.len()];
        let mut order: Vec<_> = (0..self.ranges.len()).collect();
        order.sort_unstable_by_key(|i| self.ranges[*i].start);
        let mut next = self.text.find(needle);
        for i in order {
            let range = &self.ranges[i];
            if next.is_some_and(|at| at < range.start) {
                next = self.text[range.start..]
                    .find(needle)
                    .map(|at| range.start + at);
            }
            matches[i] = next.is_some_and(|at| at >= range.start && at + needle.len() <= range.end);
        }
        matches
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn captured_text_ranges_preserve_order_overrides_unicode_scope_and_overlapping_search() {
        let tree:Snapshot=serde_json::from_value(json!({"status":"captured","capture":"1","router_revision":0,"omitted":0,"coverage":"test","elements":[
            {"id":"root","kind":"Div","metadata_truncated":false},
            {"id":"a","parent":"root","kind":"Text","text":"café ","metadata_truncated":false},
            {"id":"b","parent":"root","kind":"Div","metadata_truncated":false},
            {"id":"c","parent":"b","kind":"Text","text":"🙂","metadata_truncated":false},
            {"id":"d","parent":"root","kind":"P","text":" override","metadata_truncated":false},
            {"id":"e","parent":"d","kind":"Text","text":"ignored by parent","metadata_truncated":false}
        ]})).unwrap();
        let text = Content::new(&tree).unwrap();
        assert_eq!(text.text(0), "café 🙂 override");
        assert_eq!(text.text(2), "🙂");
        assert_eq!(text.text(5), "ignored by parent");
        assert_eq!(text.scope(2), [false, false, true, true, false, false]);
        assert!(text.contains("fé 🙂")[0]);
        assert!(!text.contains("fé 🙂")[1]);
        let overlap = Content {
            text: "aaaa".into(),
            ranges: vec![0..4, 1..4, 0..2],
            children: vec![],
            complete: vec![],
        };
        assert_eq!(overlap.contains("aaa"), [true, true, false]);
    }
    #[test]
    fn deep_ancestors_share_text_instead_of_copying_each_subtree() {
        let mut tree:Snapshot=serde_json::from_value(json!({"status":"captured","capture":"1","router_revision":0,"omitted":0,"coverage":"test","elements":[{"id":"0","kind":"Div","metadata_truncated":false}]})).unwrap();
        let template = tree.elements[0].clone();
        for i in 1..2048 {
            let mut node = template.clone();
            node.id = i.to_string();
            node.parent = Some((i - 1).to_string());
            tree.elements.push(node);
        }
        tree.elements.last_mut().unwrap().text = Some("🙂".repeat(4096));
        let content = Content::new(&tree).unwrap();
        assert_eq!(content.text.len(), 16384);
        assert_eq!(content.text(0), content.text(2047));
        assert!(content.contains("🙂🙂").into_iter().all(|matched| matched));
    }

    #[test]
    fn cycles_are_rejected_and_omissions_never_claim_complete_text() {
        let mut tree:Snapshot=serde_json::from_value(json!({"status":"captured","capture":"1","router_revision":0,"omitted":1,"coverage":"test","elements":[{"id":"a","kind":"Text","text":"x","metadata_truncated":false}]})).unwrap();
        assert!(!Content::new(&tree).unwrap().complete(0));
        tree.elements[0].parent = Some("a".into());
        assert!(Content::new(&tree).is_err());
    }
}
