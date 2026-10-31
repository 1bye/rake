//! Draw-pinned native index reads and bounded semantic projections for agents.

use super::{TIMEOUT, captures::Captures, dom::Dom, wire::Error};
use rake_client::local::Instance;
use rake_protocol::{
    control::{Query, Reply},
    inspection::{MAX_ELEMENTS, Snapshot, TREE_PAGE_SIZE, TreeRevision, WindowPage},
    router::WindowId,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

pub(super) struct Presentation {
    pub window: WindowId,
    pub frame: String,
    pub revision: TreeRevision,
    pub tree: Arc<Snapshot>,
}

pub(super) fn read(
    instance: &Instance,
    cache: &Captures,
    window: WindowId,
) -> Result<Presentation, Error> {
    if !instance.info.queries.iter().any(|q| q == "inspect_window") {
        return Err(Error::new(
            -32601,
            "Runtime does not support window-specific inspection",
        ));
    }
    let page = |presentation: Option<String>, offset| -> Result<WindowPage, Error> {
        let Reply::InspectionWindow(page) = crate::terminal::query(
            instance,
            Query::InspectWindow {
                window,
                presentation,
                offset,
            },
            TIMEOUT,
        )
        .map_err(|e| Error::new(-32000, e.to_string()))?
        else {
            return Err(Error::new(-32000, "Unexpected window inspection reply"));
        };
        Ok(page)
    };
    let first = page(None, 0)?;
    let revision = TreeRevision {
        capture: first.page.snapshot.capture.clone(),
        metadata: Some(first.metadata_revision.clone()),
        status: first.page.snapshot.status,
    };
    if first.window != window {
        return Err(Error::new(
            -32000,
            "Inspection reply targets another window",
        ));
    }
    let first_count = first.page.snapshot.elements.len();
    if first.page.total > MAX_ELEMENTS
        || first_count > TREE_PAGE_SIZE
        || first_count > first.page.total
        || first
            .page
            .next
            .is_some_and(|next| next != first_count || next >= first.page.total || first_count == 0)
        || (first.page.next.is_none() && first_count != first.page.total)
        || first.frame_sequence.parse::<u64>().is_err()
    {
        return Err(Error::new(-32000, "Invalid first window inspection page"));
    }
    if let Some(tree) = cache.find(instance.info.runtime_id.0, &revision) {
        if tree.elements.len() != first.page.total {
            return Err(Error::new(
                -32000,
                "Window index total changed without a new revision",
            ));
        }
        return Ok(Presentation {
            window,
            frame: first.frame_sequence,
            revision,
            tree,
        });
    }
    let lease = first.presentation.clone();
    let frame = first.frame_sequence.clone();
    let total = first.page.total;
    let mut next_page = first;
    let mut combined: Option<Snapshot> = None;
    loop {
        let offset = combined.as_ref().map_or(0, |tree| tree.elements.len());
        let count = next_page.page.snapshot.elements.len();
        let received = offset + count;
        if next_page.window != window
            || next_page.presentation != lease
            || next_page.frame_sequence != frame
            || next_page.metadata_revision != revision.metadata.as_deref().unwrap_or_default()
            || next_page.page.snapshot.capture != revision.capture
            || next_page.page.snapshot.status != revision.status
            || next_page.page.total != total
            || total > MAX_ELEMENTS
            || count > TREE_PAGE_SIZE
            || received > total
            || next_page
                .page
                .next
                .is_some_and(|n| n != received || n >= total || count == 0)
            || (next_page.page.next.is_none() && received != total)
        {
            return Err(Error::new(
                -32000,
                "Incomplete or inconsistent window inspection page",
            ));
        }
        let next = next_page.page.next;
        if let Some(tree) = &mut combined {
            tree.elements.extend(next_page.page.snapshot.elements);
        } else {
            combined = Some(next_page.page.snapshot);
        }
        if let Some(offset) = next {
            next_page = page(Some(lease.clone()), offset)?;
        } else {
            break;
        }
    }
    let tree = Arc::new(combined.expect("first page supplies metadata"));
    cache.retain(instance.info.runtime_id.0, &revision, Arc::clone(&tree));
    Ok(Presentation {
        window,
        frame,
        revision,
        tree,
    })
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Selector {
    pub node_id: Option<i32>,
    pub stable_id: Option<String>,
    pub id: Option<String>,
    pub role: Option<String>,
    pub name: Option<String>,
    pub text_contains: Option<String>,
    pub text: Option<String>,
    pub bounds: Option<Bounds>,
    pub enabled: Option<bool>,
    pub visible: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Bounds {
    x: Option<f32>,
    y: Option<f32>,
    width: Option<f32>,
    height: Option<f32>,
    #[serde(default = "default_tolerance")]
    tolerance: f32,
}
fn default_tolerance() -> f32 {
    0.5
}
impl Bounds {
    fn valid(&self) -> bool {
        self.tolerance.is_finite()
            && (0. ..=100.).contains(&self.tolerance)
            && [self.x, self.y, self.width, self.height]
                .into_iter()
                .flatten()
                .all(f32::is_finite)
            && self.width.is_none_or(|v| v >= 0.)
            && self.height.is_none_or(|v| v >= 0.)
    }
    fn matches(&self, bounds: Option<rake_protocol::inspection::Layout>) -> bool {
        bounds.is_some_and(|b| {
            [
                (self.x, b.x),
                (self.y, b.y),
                (self.width, b.width),
                (self.height, b.height),
            ]
            .into_iter()
            .all(|(expected, actual)| {
                expected.is_none_or(|value| (value - actual).abs() <= self.tolerance)
            })
        })
    }
}

pub(super) fn selector(params: &Value) -> Result<Selector, Error> {
    let query = params.get("query").cloned().unwrap_or_else(|| json!({}));
    if serde_json::to_vec(&query).map_or(true, |v| v.len() > 4096) {
        return Err(Error::new(-32602, "Semantic query exceeds 4 KiB"));
    }
    let selector: Selector =
        serde_json::from_value(query).map_err(|e| Error::new(-32602, e.to_string()))?;
    if selector.bounds.as_ref().is_some_and(|b| !b.valid()) {
        return Err(Error::new(-32602, "Invalid bounds predicate"));
    }
    Ok(selector)
}

fn prefix(value: &str) -> &str {
    let mut end = value.len().min(512);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

pub(super) fn project(
    presentation: &Presentation,
    dom: &Dom,
    params: &Value,
) -> Result<Value, Error> {
    let selector = selector(params)?;
    let number = |name: &str, default: usize, max: usize| match params.get(name) {
        None => Ok(default),
        Some(v) => v
            .as_u64()
            .filter(|n| *n <= max as u64)
            .map(|n| n as usize)
            .ok_or_else(|| Error::new(-32602, format!("Invalid {name}"))),
    };
    let limit = number("limit", 100, 256)?;
    let offset = number("offset", 0, MAX_ELEMENTS)?;
    if limit == 0 {
        return Err(Error::new(-32602, "limit must be within 1–256"));
    }
    let revision =
        json!({"capture":presentation.revision.capture,"metadata":presentation.revision.metadata});
    if params
        .get("expectedRevision")
        .is_some_and(|expected| *expected != revision)
        || (offset > 0 && params.get("expectedRevision").is_none())
    {
        return Err(Error::new(
            -32000,
            "Snapshot revision changed or is missing; restart pagination",
        ));
    }
    let mut matches = 0usize;
    let mut elements = Vec::new();
    for element in &presentation.tree.elements {
        let node = dom
            .node_id(&element.id)
            .ok_or_else(|| Error::new(-32000, "Semantic node is no longer retained"))?;
        let ax = element.accessibility.as_ref();
        let name = ax.and_then(|a| a.name.as_deref().or(a.label.as_deref()));
        let role = ax.and_then(|a| a.role.as_deref());
        let enabled = ax.map(|a| !a.disabled);
        let visible = element.layout.map(|b| {
            b.width > 0.
                && b.height > 0.
                && !ax.is_some_and(|a| a.hidden)
                && element
                    .content_mask
                    .is_none_or(|b| b.width > 0. && b.height > 0.)
        });
        if selector.node_id.is_some_and(|v| v != node)
            || selector
                .stable_id
                .as_ref()
                .is_some_and(|v| element.stable_id.as_ref() != Some(v))
            || selector
                .id
                .as_ref()
                .is_some_and(|v| element.semantic_id.as_ref() != Some(v))
            || selector
                .role
                .as_ref()
                .is_some_and(|v| !role.is_some_and(|role| role.eq_ignore_ascii_case(v)))
            || selector
                .name
                .as_ref()
                .is_some_and(|v| name != Some(v.as_str()))
            || selector
                .text_contains
                .as_ref()
                .is_some_and(|v| !element.text.as_deref().is_some_and(|text| text.contains(v)))
            || selector
                .text
                .as_ref()
                .is_some_and(|v| element.text.as_ref() != Some(v))
            || selector
                .bounds
                .as_ref()
                .is_some_and(|v| !v.matches(element.layout))
            || selector.enabled.is_some_and(|v| enabled != Some(v))
            || selector.visible.is_some_and(|v| visible != Some(v))
        {
            continue;
        }
        matches += 1;
        if matches <= offset || elements.len() == limit {
            continue;
        }
        elements.push(json!({"nodeId":node,"stableId":element.stable_id,"id":element.semantic_id,
            "parentId":element.parent.as_deref().and_then(|p|dom.node_id(p)),"type":element.display_name(),
            "role":role,"name":name.map(prefix),"nameTruncated":name.is_some_and(|n|n.len()>512),"text":element.text.as_deref().map(prefix),
            "textTruncated":element.text.as_ref().is_some_and(|t|t.len()>512),"bounds":element.layout,
            "visibleBounds":element.content_mask,"visible":visible,"enabled":enabled,
            "focused":ax.and_then(|a|a.focused),"states":ax.map(|a|&a.states),"source":element.source,
            "metadataTruncated":element.metadata_truncated,"styleEditable":element.style_editable,"textEditable":element.text_editable}));
    }
    Ok(
        json!({"windowId":presentation.window,"frameSequence":presentation.frame,"revision":revision,
        "elements":elements,"totalMatches":matches,"offset":offset,"hasMore":matches>offset+limit,
        "omitted":presentation.tree.omitted,"coverage":presentation.tree.coverage,
        "visibilitySemantics":"sampled layout, clipping and accessibility; final occlusion is unavailable"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn geometry_predicates_have_explicit_finite_tolerances() {
        let bounds: Bounds = serde_json::from_value(json!({"width":10.2})).unwrap();
        assert!(bounds.matches(Some(rake_protocol::inspection::Layout {
            x: 0.,
            y: 0.,
            width: 10.,
            height: 5.
        })));
        assert!(!bounds.matches(None));
        assert!(selector(&json!({"query":{"bounds":{"width":-1}}})).is_err());
        assert!(selector(&json!({"query":{"bounds":{"tolerance":101}}})).is_err());
        assert!(selector(&json!({"query":{"bounds":{"width":1e300}}})).is_err());
    }
    #[test]
    fn semantic_queries_preserve_identity_and_bound_unicode_and_pagination() {
        let tree:Snapshot=serde_json::from_value(json!({"status":"captured","capture":"1","router_revision":0,"omitted":0,"coverage":"test","elements":[{"id":"one","stable_id":"s1","kind":"Button","semantic_id":"save","metadata_truncated":false,"text":"🙂".repeat(200)}]})).unwrap();
        let mut dom = Dom::default();
        dom.update(tree.clone()).unwrap();
        let p = Presentation {
            window: WindowId(1),
            frame: "2".into(),
            revision: TreeRevision {
                capture: "1".into(),
                metadata: Some("1".into()),
                status: tree.status,
            },
            tree: Arc::new(tree),
        };
        let result = project(&p, &dom, &json!({"query":{"id":"save"},"limit":1})).unwrap();
        assert_eq!(result["totalMatches"], 1);
        assert_eq!(result["elements"][0]["stableId"], "s1");
        assert_eq!(result["elements"][0]["text"].as_str().unwrap().len(), 512);
        assert_eq!(result["elements"][0]["textTruncated"], true);
        assert!(project(&p, &dom, &json!({"offset":1})).is_err());
        assert!(project(&p, &dom, &json!({"query":{"unknown":true}})).is_err());
        assert_eq!(
            project(&p, &dom, &json!({"query":{"id":"absent"}})).unwrap()["totalMatches"],
            0
        );
    }
}
