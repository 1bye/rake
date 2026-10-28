//! Read-only DOM/CSS projection of one bounded Rake capture. Node IDs never alias captures.

use super::wire::Error;
use rake::inspection::{Element, Snapshot};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct Dom {
    snapshot: Option<Snapshot>,
    nodes: BTreeMap<i32, Element>,
    next: i32,
    omitted_id: i32,
    sent: BTreeSet<i32>,
    children_sent: BTreeSet<i32>,
    style_updates: BTreeSet<i32>,
}

impl Dom {
    pub fn update(&mut self, snapshot: Snapshot) -> Result<bool, Error> {
        if self
            .snapshot
            .as_ref()
            .is_some_and(|old| old.capture == snapshot.capture && old.status == snapshot.status)
        {
            for (&id, element) in &mut self.nodes {
                if let Some(current) = snapshot
                    .elements
                    .iter()
                    .find(|current| current.id == element.id)
                {
                    if (current.styles != element.styles || current.resolved != element.resolved)
                        && self.sent.contains(&id)
                    {
                        self.style_updates.insert(id);
                    }
                    *element = current.clone();
                }
            }
            self.snapshot = Some(snapshot);
            return Ok(false);
        }
        let changed = self.snapshot.is_some();
        self.nodes.clear();
        self.sent.clear();
        self.children_sent.clear();
        self.style_updates.clear();
        self.next = self.next.max(2);
        for element in &snapshot.elements {
            let id = self.next;
            self.next = self
                .next
                .checked_add(1)
                .ok_or_else(|| Error::new(-32000, "CDP node identity exhausted; reconnect"))?;
            self.nodes.insert(id, element.clone());
        }
        self.omitted_id = self.next;
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| Error::new(-32000, "CDP node identity exhausted"))?;
        self.snapshot = Some(snapshot);

        Ok(changed)
    }

    pub fn generation(&self) -> Option<&str> {
        self.snapshot.as_ref().map(|s| s.capture.as_str())
    }

    pub fn take_style_updates(&mut self) -> Vec<i32> {
        std::mem::take(&mut self.style_updates)
            .into_iter()
            .collect()
    }

    pub fn style_sheet_text(&self) -> String {
        let mut text = String::from("/* Read-only GPUI resolved style snapshots. */\n");
        for (&id, element) in &self.nodes {
            let style = super::styles::inline(visual_style(element));
            text.push_str(&format!(
                "/* {} */\n[data-rake-node=\"{id}\"] {{\n{}\n}}\n",
                element.id,
                style["cssText"].as_str().unwrap_or_default()
            ));
        }
        text
    }

    pub fn node_id(&self, id: &str) -> Option<i32> {
        self.nodes
            .iter()
            .find(|(_, e)| e.id == id)
            .map(|(&id, _)| id)
    }

    pub fn overlay_command(
        &self,
        method: &str,
        params: &Value,
    ) -> Result<rake::control::Command, Error> {
        use rake::control::Command;
        match method {
            "Overlay.highlightNode" => {
                let id = params["nodeId"]
                    .as_i64()
                    .or_else(|| params["backendNodeId"].as_i64())
                    .and_then(|id| i32::try_from(id).ok())
                    .ok_or_else(|| Error::new(-32602, "nodeId or backendNodeId required"))?;
                let element = self.element(id)?;
                Ok(Command::HighlightElement {
                    id: Some(element.id.clone()),
                    generation: self.generation().unwrap_or_default().into(),
                })
            }
            "Overlay.hideHighlight" => Ok(Command::HighlightElement {
                id: None,
                generation: String::new(),
            }),
            "Overlay.setInspectMode" => match params["mode"].as_str() {
                Some("searchForNode") => Ok(Command::InspectMode {
                    enabled: true,
                    lease_ms: 1000,
                }),
                Some("none") => Ok(Command::InspectMode {
                    enabled: false,
                    lease_ms: 0,
                }),
                _ => Err(Error::new(
                    -32602,
                    "Only searchForNode and none inspect modes are supported",
                )),
            },
            _ => Err(Error::new(-32601, "Unknown Overlay method")),
        }
    }

    pub fn clear(&mut self) {
        self.snapshot = None;
        self.nodes.clear();
        self.sent.clear();
        self.children_sent.clear();
        self.style_updates.clear();
    }

    fn element(&self, id: i32) -> Result<&Element, Error> {
        self.nodes
            .get(&id)
            .ok_or_else(|| Error::new(-32000, "Node is stale or unavailable; refresh the document"))
    }

    fn children(&self, id: i32) -> Vec<i32> {
        if id != 1 && !self.nodes.contains_key(&id) {
            return Vec::new();
        }
        let parent = self.nodes.get(&id).map(|element| element.id.as_str());
        let mut children: Vec<_> = self
            .nodes
            .iter()
            .filter_map(|(&child_id, element)| {
                let belongs = if id == 1 {
                    element
                        .parent
                        .as_deref()
                        .is_none_or(|parent| self.node_id(parent).is_none())
                } else {
                    element.parent.as_deref() == parent
                };
                belongs.then_some(child_id)
            })
            .collect();
        if id == 1 && self.snapshot.as_ref().is_some_and(|s| s.omitted > 0) {
            children.push(self.omitted_id);
        }
        children
    }

    fn node(&self, id: i32, depth: usize) -> Value {
        self.node_with_seen(id, depth, &mut BTreeSet::new())
    }

    fn node_with_seen(&self, id: i32, depth: usize, seen: &mut BTreeSet<i32>) -> Value {
        if !seen.insert(id) {
            return Value::Null;
        }
        if id == self.omitted_id {
            return json!({"nodeId":id,"backendNodeId":id,"nodeType":8,"nodeName":"#comment","localName":"","nodeValue":format!("{} elements omitted", self.snapshot.as_ref().map_or(0, |s| s.omitted))});
        }
        let children = self.children(id);
        let mut node = if id == 1 {
            json!({"nodeId":1,"backendNodeId":1,"nodeType":9,"nodeName":"#document","localName":"","nodeValue":"",
                "documentURL":"rake://runtime","baseURL":"rake://runtime"})
        } else {
            let element = &self.nodes[&id];
            let name = rake::inspection::display_name(&element.kind);
            json!({"nodeId":id,"backendNodeId":id,"nodeType":1,"nodeName":name,"localName":name,
                "nodeValue":"","attributes":attributes(element)})
        };
        node["childNodeCount"] = json!(children.len());
        if depth > 0 || children.is_empty() {
            node["children"] = json!(
                children
                    .into_iter()
                    .map(|child| self.node_with_seen(child, depth.saturating_sub(1), seen))
                    .filter(|node| !node.is_null())
                    .collect::<Vec<_>>()
            );
        }
        node
    }

    fn remember(&mut self, node: &Value) {
        let id = node["nodeId"].as_i64().unwrap() as i32;
        self.sent.insert(id);
        if let Some(children) = node["children"].as_array() {
            self.children_sent.insert(id);
            for child in children {
                self.remember(child);
            }
        }
    }

    pub fn document(&mut self, params: &Value) -> Result<Value, Error> {
        let depth = depth(params, 1)?;
        self.sent.clear();
        self.children_sent.clear();
        let root = self.node(1, depth);
        self.remember(&root);
        Ok(root)
    }

    fn send_children(&mut self, id: i32, depth: usize, events: &mut Vec<Value>) {
        if depth == 0 {
            return;
        }
        let children = self.children(id);
        if self.children_sent.insert(id) {
            let nodes: Vec<_> = children
                .iter()
                .map(|&child| self.node(child, depth - 1))
                .collect();
            for node in &nodes {
                self.remember(node);
            }
            events.push(json!({"parentId":id,"nodes":nodes}));
        } else {
            for child in children {
                self.send_children(child, depth - 1, events);
            }
        }
    }

    pub fn request_children(&mut self, params: &Value) -> Result<Vec<Value>, Error> {
        let id = node_id(params)?;
        let depth = depth(params, 1)?;
        if !self.sent.contains(&id) {
            return Err(Error::new(
                -32000,
                "Parent node is not known; request the document first",
            ));
        }
        let mut events = Vec::new();
        self.send_children(id, depth, &mut events);
        Ok(events)
    }

    fn send_path(&mut self, mut id: i32, events: &mut Vec<Value>) -> Result<(), Error> {
        let mut parents = Vec::new();
        let mut seen = BTreeSet::new();
        while !self.sent.contains(&id) {
            if !seen.insert(id) {
                return Err(Error::new(-32000, "Captured ancestry contains a cycle"));
            }
            id = self
                .nodes
                .get(&id)
                .and_then(|element| element.parent.as_deref())
                .and_then(|parent| self.node_id(parent))
                .unwrap_or(1);
            parents.push(id);
        }
        for parent in parents.into_iter().rev() {
            self.send_children(parent, 1, events);
        }
        Ok(())
    }

    pub fn reveal_node(&mut self, id: i32) -> Result<Option<Vec<Value>>, Error> {
        if !self.sent.contains(&1) {
            return Ok(None);
        }
        let mut events = Vec::new();
        self.send_path(id, &mut events)?;
        Ok(Some(events))
    }

    pub fn push_nodes(&mut self, params: &Value) -> Result<(Value, Vec<Value>), Error> {
        let ids = params["backendNodeIds"]
            .as_array()
            .ok_or_else(|| Error::new(-32602, "backendNodeIds required"))?;
        if !self.sent.contains(&1) {
            return Err(Error::new(-32000, "Request the document first"));
        }
        let mut events = Vec::new();
        let nodes: Vec<_> = ids
            .iter()
            .map(|value| {
                let id = value
                    .as_i64()
                    .and_then(|id| i32::try_from(id).ok())
                    .unwrap_or(0);
                if id == 1 || self.nodes.contains_key(&id) {
                    self.send_path(id, &mut events)?;
                    Ok(id)
                } else {
                    Ok(0)
                }
            })
            .collect::<Result<_, Error>>()?;
        Ok((json!({"nodeIds":nodes}), events))
    }

    pub fn handle(&self, method: &str, params: &Value) -> Result<Value, Error> {
        let id = node_id(params)?;
        if method == "DOM.describeNode" {
            if id != 1 {
                self.element(id)?;
            }
            return Ok(json!({"node":self.node(id, depth(params, 0)?)}));
        }
        let element = self.element(id)?;
        match method {
            "DOM.getAttributes" => Ok(json!({"attributes":attributes(element)})),
            "DOM.getBoxModel" => {
                let layout = element
                    .layout
                    .ok_or_else(|| Error::new(-32000, "Layout is not captured for this node"))?;
                let (x, y, w, h) = (layout.x, layout.y, layout.width, layout.height);
                let quad = json!([x, y, x + w, y, x + w, y + h, x, y + h]);
                Ok(
                    json!({"model":{"content":quad,"padding":quad,"border":quad,"margin":quad,"width":w,"height":h}}),
                )
            }
            "CSS.getMatchedStylesForNode" | "CSS.getInlineStylesForNode" => {
                let mut inherited = Vec::new();
                let mut parent = element.parent.as_deref();
                let mut seen = BTreeSet::new();
                while let Some(ancestor) = parent
                    .and_then(|parent| self.node_id(parent))
                    .and_then(|id| self.nodes.get(&id))
                {
                    if !seen.insert(&ancestor.id) {
                        break;
                    }
                    inherited.push(json!({"inlineStyle":super::styles::inline(ancestor.styles.as_ref()),"matchedCSSRules":[]}));
                    parent = ancestor.parent.as_deref();
                }
                Ok(
                    json!({"inlineStyle":super::styles::inline(visual_style(element)),"matchedCSSRules":[],"inherited":inherited,"pseudoElements":[]}),
                )
            }
            "CSS.getComputedStyleForNode" => {
                let layout = element
                    .layout
                    .ok_or_else(|| Error::new(-32000, "Layout is not captured for this node"))?;
                let mut properties = super::styles::properties(
                    element
                        .resolved
                        .as_ref()
                        .map(|resolved| &resolved.properties),
                );
                properties
                    .retain(|property| property["name"] != "width" && property["name"] != "height");
                properties.extend([
                    json!({"name":"width","value":format!("{}px",layout.width)}),
                    json!({"name":"height","value":format!("{}px",layout.height)}),
                ]);
                Ok(json!({"computedStyle":properties}))
            }
            _ => Err(Error::new(
                -32601,
                format!("Unsupported CDP method: {method}"),
            )),
        }
    }
}

fn visual_style(element: &Element) -> Option<&Value> {
    element
        .resolved
        .as_ref()
        .map(|resolved| &resolved.properties)
        .or(element.styles.as_ref())
}

fn attributes(element: &Element) -> Vec<String> {
    let mut attrs = Vec::new();
    if let Some(id) = &element.semantic_id {
        attrs.extend(["id".into(), id.clone()]);
    }
    attrs
}

fn node_id(params: &Value) -> Result<i32, Error> {
    params["nodeId"]
        .as_i64()
        .or_else(|| params["backendNodeId"].as_i64())
        .and_then(|id| i32::try_from(id).ok())
        .ok_or_else(|| Error::new(-32602, "nodeId required"))
}

fn depth(params: &Value, default: i64) -> Result<usize, Error> {
    let depth = if params.get("depth").is_some() {
        params["depth"]
            .as_i64()
            .ok_or_else(|| Error::new(-32602, "depth must be an integer"))?
    } else {
        default
    };
    match depth {
        -1 => Ok(rake::inspection::MAX_ELEMENTS + 1),
        0.. => Ok((depth as usize).min(rake::inspection::MAX_ELEMENTS + 1)),
        _ => Err(Error::new(-32602, "depth must be -1 or nonnegative")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn capture(id: &str) -> Snapshot {
        serde_json::from_value(json!({"status":"captured","capture":id,"router_revision":1,"omitted":2,"coverage":"test","elements":[
            {"id":format!("{id}-a"),"parent":null,"kind":"gpui::Stateful<gpui::elements::div::Div>","semantic_id":"root","source":{"file":"app.rs","line":3,"column":4},"layout":{"x":1.,"y":2.,"width":10.,"height":20.},"styles":{"display":"Flex","size":{"width":"100%"},"background":{"Color":"opaque"}},"metadata_truncated":false,"tab_id":null,"entry_id":null},
            {"id":format!("{id}-b"),"parent":format!("{id}-a"),"kind":"gpui::ViewElement<app::Button<app::Action>>","metadata_truncated":false}
        ]})).unwrap()
    }
    #[test]
    fn tree_styles_bounds_and_stale_node_ids() {
        let mut dom = Dom::default();
        dom.update(capture("a")).unwrap();
        let doc = dom.document(&json!({"depth":-1})).unwrap();
        let root = &doc["children"][0];
        assert_eq!(root["attributes"], json!(["id", "root"]));
        assert_eq!(root["nodeName"], "Div");
        assert_eq!(root["localName"], "Div");
        assert_eq!(root["children"][0]["nodeName"], "Button");
        assert_eq!(
            dom.nodes.values().next().unwrap().kind,
            "gpui::Stateful<gpui::elements::div::Div>"
        );
        assert_eq!(doc["children"][1]["nodeValue"], "2 elements omitted");
        let params = json!({"nodeId":root["nodeId"]});
        assert_eq!(
            dom.handle("DOM.getBoxModel", &params).unwrap()["model"]["content"],
            json!([1., 2., 11., 2., 11., 22., 1., 22.])
        );
        let css = dom.handle("CSS.getMatchedStylesForNode", &params).unwrap();
        assert!(
            css["inlineStyle"]["cssProperties"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["name"] == "--gpui-background-color")
        );
        assert_eq!(
            dom.handle("CSS.getComputedStyleForNode", &params).unwrap()["computedStyle"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert!(dom.update(capture("b")).unwrap());
        assert!(dom.handle("DOM.getBoxModel", &params).is_err());
    }
    #[test]
    fn expansion_and_picker_delivery_never_replace_known_nodes() {
        let mut dom = Dom::default();
        dom.update(capture("one")).unwrap();
        let root_id = dom.node_id("one-a").unwrap();
        let child_id = dom.node_id("one-b").unwrap();
        let doc = dom.document(&json!({})).unwrap();
        assert_eq!(doc["children"][0]["childNodeCount"], 1);
        assert!(doc["children"][0].get("children").is_none());
        assert!(dom.request_children(&json!({"nodeId":child_id})).is_err());

        let events = dom.request_children(&json!({"nodeId":root_id})).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["nodes"][0]["nodeId"], child_id);
        assert!(
            dom.request_children(&json!({"nodeId":root_id}))
                .unwrap()
                .is_empty()
        );
        let (result, events) = dom
            .push_nodes(&json!({"backendNodeIds":[child_id,child_id,99999]}))
            .unwrap();
        assert_eq!(result["nodeIds"], json!([child_id, child_id, 0]));
        assert!(
            events.is_empty(),
            "picking a known node must not recreate Chrome DOM objects"
        );

        dom.document(&json!({})).unwrap();
        let (_, events) = dom
            .push_nodes(&json!({"backendNodeIds":[child_id]}))
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["parentId"], root_id);
        assert!(
            dom.request_children(&json!({"nodeId":root_id,"depth":-1}))
                .unwrap()
                .is_empty()
        );
        dom.update(capture("two")).unwrap();
        assert!(dom.request_children(&json!({"nodeId":root_id})).is_err());
        assert!(dom.document(&json!({"depth":-2})).is_err());
        assert!(dom.document(&json!({"depth":"all"})).is_err());
    }

    #[test]
    fn deeper_requests_extend_the_frontier_and_inherited_styles_follow_ancestors() {
        let mut snapshot = capture("nested");
        snapshot.omitted = 0;
        snapshot.elements[0].styles =
            Some(json!({"text":{"color":{"hue":0.,"saturation":1.,"lightness":0.5,"alpha":1.}}}));
        let mut grandchild = snapshot.elements[1].clone();
        grandchild.id = "nested-c".into();
        grandchild.parent = Some("nested-b".into());
        snapshot.elements.push(grandchild);
        let mut dom = Dom::default();
        dom.update(snapshot).unwrap();
        let root = dom.node_id("nested-a").unwrap();
        let child = dom.node_id("nested-b").unwrap();
        let leaf = dom.node_id("nested-c").unwrap();
        dom.document(&json!({"depth":2})).unwrap();
        let events = dom
            .request_children(&json!({"nodeId":root,"depth":-1}))
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["parentId"], child);
        assert_eq!(events[0]["nodes"][0]["nodeId"], leaf);
        let styles = dom
            .handle("CSS.getMatchedStylesForNode", &json!({"nodeId":leaf}))
            .unwrap();
        assert_eq!(styles["inherited"].as_array().unwrap().len(), 2);
        assert_eq!(
            styles["inherited"][1]["inlineStyle"]["cssProperties"][0]["name"],
            "color"
        );
        assert_eq!(
            styles["inherited"][1]["inlineStyle"]["cssProperties"][0]["value"],
            "#ff0000"
        );
        assert_eq!(
            dom.handle("DOM.getAttributes", &json!({"nodeId":root}))
                .unwrap()["attributes"],
            json!(["id", "root"])
        );
        assert!(
            dom.element(root).unwrap().source.is_some(),
            "source metadata remains available to Rake"
        );
    }

    #[test]
    fn resolved_style_changes_refresh_css_without_replacing_node_ids() {
        let mut snapshot = capture("live");
        snapshot.elements[0].resolved = Some(rake::inspection::ResolvedStyle {
            properties: json!({"background":{"Color":{"tag":"Solid","solid":{"h":0.,"s":1.,"l":0.5,"a":1.}}},"text":{"font_size":"24px"}}),
            hovered: false,
        });
        let mut dom = Dom::default();
        dom.update(snapshot.clone()).unwrap();
        dom.document(&json!({"depth":-1})).unwrap();
        let id = dom.node_id("live-a").unwrap();
        snapshot.elements[0].resolved.as_mut().unwrap().properties["background"]["Color"]["solid"]
            ["h"] = json!(0.5);
        snapshot.elements[0].resolved.as_mut().unwrap().hovered = true;
        assert!(!dom.update(snapshot).unwrap());
        assert_eq!(dom.node_id("live-a"), Some(id));
        assert_eq!(dom.take_style_updates(), vec![id]);
        let computed = dom
            .handle("CSS.getComputedStyleForNode", &json!({"nodeId":id}))
            .unwrap();
        let get = |name| {
            computed["computedStyle"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["name"] == name)
                .unwrap()["value"]
                .as_str()
                .unwrap()
        };
        assert_eq!(get("background-color"), "#00ffff");
        assert_eq!(get("font-size"), "24px");
        assert_eq!(get("width"), "10px");
        assert!(
            dom.push_nodes(&json!({"backendNodeIds":[id]}))
                .unwrap()
                .1
                .is_empty()
        );
        assert!(dom.style_sheet_text().contains("#00ffff"));
    }

    #[test]
    fn overlay_commands_use_exact_capture_ids_and_shared_backend_ids() {
        use rake::control::Command;
        let mut dom = Dom::default();
        dom.update(capture("test")).unwrap();
        let id = dom.node_id("test-a").unwrap();
        for params in [json!({"nodeId":id}), json!({"backendNodeId":id})] {
            assert_eq!(
                dom.overlay_command("Overlay.highlightNode", &params)
                    .unwrap(),
                Command::HighlightElement {
                    id: Some("test-a".into()),
                    generation: "test".into()
                }
            );
        }
        assert_eq!(
            dom.overlay_command("Overlay.hideHighlight", &json!({}))
                .unwrap(),
            Command::HighlightElement {
                id: None,
                generation: String::new()
            }
        );
        assert_eq!(
            dom.overlay_command("Overlay.setInspectMode", &json!({"mode":"searchForNode"}))
                .unwrap(),
            Command::InspectMode {
                enabled: true,
                lease_ms: 1000
            }
        );
        assert_eq!(
            dom.overlay_command("Overlay.setInspectMode", &json!({"mode":"none"}))
                .unwrap(),
            Command::InspectMode {
                enabled: false,
                lease_ms: 0
            }
        );
        assert!(
            dom.overlay_command("Overlay.setInspectMode", &json!({"mode":"invalid"}))
                .is_err()
        );
        dom.update(capture("replacement")).unwrap();
        assert!(
            dom.overlay_command("Overlay.highlightNode", &json!({"nodeId":id}))
                .is_err()
        );
    }
}
