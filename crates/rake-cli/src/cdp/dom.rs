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
}

impl Dom {
    pub fn update(&mut self, snapshot: Snapshot) -> Result<bool, Error> {
        if self
            .snapshot
            .as_ref()
            .is_some_and(|old| old.capture == snapshot.capture && old.status == snapshot.status)
        {
            return Ok(false);
        }
        let changed = self.snapshot.is_some();
        self.nodes.clear();
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

    pub fn clear(&mut self) {
        self.snapshot = None;
        self.nodes.clear();
    }

    fn element(&self, id: i32) -> Result<&Element, Error> {
        self.nodes
            .get(&id)
            .ok_or_else(|| Error::new(-32000, "Node is stale or unavailable; refresh the document"))
    }

    pub fn document(&self) -> Value {
        let known: BTreeSet<_> = self.nodes.values().map(|e| e.id.as_str()).collect();
        let mut seen = BTreeSet::new();
        let mut children = Vec::new();
        for (&id, element) in &self.nodes {
            if element
                .parent
                .as_deref()
                .is_none_or(|parent| !known.contains(parent))
            {
                children.push(self.node(id, &mut seen));
            }
        }
        let omitted = self.snapshot.as_ref().map(|s| s.omitted).unwrap_or(0);
        if omitted > 0 {
            children.push(json!({"nodeId":self.omitted_id,"backendNodeId":self.omitted_id,"nodeType":8,"nodeName":"#comment","localName":"","nodeValue":format!("{omitted} elements omitted")}));
        }

        json!({"nodeId":1,"backendNodeId":1,"nodeType":9,"nodeName":"#document","localName":"","nodeValue":"",
            "documentURL":"rake://runtime","baseURL":"rake://runtime","childNodeCount":children.len(),"children":children})
    }

    fn node(&self, id: i32, seen: &mut BTreeSet<i32>) -> Value {
        let element = &self.nodes[&id];
        if !seen.insert(id) {
            return Value::Null;
        }
        let children: Vec<_> = self
            .nodes
            .iter()
            .filter(|(_, child)| child.parent.as_deref() == Some(element.id.as_str()))
            .map(|(&id, _)| self.node(id, seen))
            .filter(|v| !v.is_null())
            .collect();

        json!({"nodeId":id,"backendNodeId":id,"nodeType":1,"nodeName":element.kind,"localName":element.kind,
            "nodeValue":"","attributes":attributes(element),"childNodeCount":children.len(),"children":children})
    }

    pub fn handle(&self, method: &str, params: &Value) -> Result<Value, Error> {
        let id = params["nodeId"]
            .as_i64()
            .or_else(|| params["backendNodeId"].as_i64())
            .and_then(|id| i32::try_from(id).ok());
        if method == "DOM.pushNodesByBackendIdsToFrontend" {
            let ids = params["backendNodeIds"]
                .as_array()
                .ok_or_else(|| Error::new(-32602, "backendNodeIds required"))?;
            return Ok(
                json!({"nodeIds":ids.iter().map(|id|id.as_i64().and_then(|id|i32::try_from(id).ok()).filter(|id|*id==1 || self.nodes.contains_key(id)).unwrap_or(0)).collect::<Vec<_>>()}),
            );
        }
        let id = id.ok_or_else(|| Error::new(-32602, "nodeId required"))?;
        if method == "DOM.describeNode" {
            return Ok(
                json!({"node":if id==1 {self.document()} else {self.element(id)?;self.node(id,&mut BTreeSet::new())}}),
            );
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
                let properties = styles(element.styles.as_ref());
                let text = properties
                    .iter()
                    .map(|p| {
                        format!(
                            "{}: {};",
                            p["name"].as_str().unwrap_or_default(),
                            p["value"].as_str().unwrap_or_default()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok(
                    json!({"inlineStyle":{"cssProperties":properties,"shorthandEntries":[],"cssText":text},"matchedCSSRules":[],"inherited":[],"pseudoElements":[]}),
                )
            }
            "CSS.getComputedStyleForNode" => {
                let layout = element
                    .layout
                    .ok_or_else(|| Error::new(-32000, "Layout is not captured for this node"))?;
                Ok(
                    json!({"computedStyle":[{"name":"width","value":format!("{}px",layout.width)},{"name":"height","value":format!("{}px",layout.height)}]}),
                )
            }
            _ => Err(Error::new(
                -32601,
                format!("Unsupported CDP method: {method}"),
            )),
        }
    }
}

fn attributes(element: &Element) -> Vec<String> {
    let mut attrs = Vec::new();
    if let Some(id) = &element.semantic_id {
        attrs.extend(["id".into(), id.clone()]);
    }
    if let Some(source) = &element.source {
        attrs.extend([
            "data-source".into(),
            format!("{}:{}:{}", source.file, source.line, source.column),
        ]);
    }
    attrs
}

fn styles(value: Option<&Value>) -> Vec<Value> {
    let mut properties = Vec::new();
    let Some(Value::Object(fields)) = value else {
        return properties;
    };
    for (key, value) in fields {
        if matches!(
            key.as_str(),
            "size" | "min_size" | "max_size" | "padding" | "margin" | "gap"
        ) && value.is_object()
        {
            for (side, value) in value.as_object().into_iter().flatten() {
                let name = match key.as_str() {
                    "size" => side.clone(),
                    "min_size" => format!("min-{side}"),
                    "max_size" => format!("max-{side}"),
                    "gap" => {
                        if side == "width" {
                            "column-gap".into()
                        } else {
                            "row-gap".into()
                        }
                    }
                    _ => format!("{key}-{side}"),
                };
                properties.push(property(&name, value, true));
            }
            continue;
        }
        let name = key.replace('_', "-");
        let known = matches!(
            key.as_str(),
            "display"
                | "position"
                | "flex_direction"
                | "flex_wrap"
                | "align_items"
                | "align_self"
                | "justify_content"
                | "flex_grow"
                | "flex_shrink"
                | "opacity"
                | "overflow_x"
                | "overflow_y"
        );
        properties.push(property(
            &if known { name } else { format!("-gpui-{name}") },
            value,
            known,
        ));
    }
    properties
}

fn property(name: &str, value: &Value, known: bool) -> Value {
    let value = match value.as_str() {
        Some(v) if known => match v {
            "Flex" => "flex",
            "Block" => "block",
            "None" => "none",
            "Row" => "row",
            "Column" => "column",
            "Center" => "center",
            "Start" => "start",
            "End" => "end",
            "FlexStart" => "flex-start",
            "FlexEnd" => "flex-end",
            "SpaceBetween" => "space-between",
            "SpaceAround" => "space-around",
            "Stretch" => "stretch",
            "Relative" => "relative",
            "Absolute" => "absolute",
            "Hidden" => "hidden",
            "Scroll" => "scroll",
            "Visible" => "visible",
            "Wrap" => "wrap",
            "NoWrap" => "nowrap",
            _ => v,
        }
        .to_owned(),
        Some(v) => v.to_owned(),
        None => value.to_string(),
    };
    json!({"name":name,"value":value,"implicit":false,"parsedOk":known,"disabled":false})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn capture(id: &str) -> Snapshot {
        serde_json::from_value(json!({"status":"captured","capture":id,"router_revision":1,"omitted":2,"coverage":"test","elements":[
            {"id":format!("{id}-a"),"parent":null,"kind":"Div","semantic_id":"root","source":{"file":"app.rs","line":3,"column":4},"layout":{"x":1.,"y":2.,"width":10.,"height":20.},"styles":{"display":"Flex","size":{"width":"100%"},"background":{"Color":"opaque"}},"metadata_truncated":false,"tab_id":null,"entry_id":null},
            {"id":format!("{id}-b"),"parent":format!("{id}-a"),"kind":"Button","metadata_truncated":false}
        ]})).unwrap()
    }
    #[test]
    fn tree_styles_bounds_and_stale_node_ids() {
        let mut dom = Dom::default();
        dom.update(capture("a")).unwrap();
        let doc = dom.document();
        let root = &doc["children"][0];
        assert_eq!(
            root["attributes"],
            json!(["id", "root", "data-source", "app.rs:3:4"])
        );
        assert_eq!(root["children"][0]["nodeName"], "Button");
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
                .any(|p| p["name"] == "-gpui-background")
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
}
