//! Incremental DOM/CSS projection. Presentation identities survive consecutive captures.

use super::wire::{Error, Event};
use rake_protocol::inspection::{Element, Snapshot};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct Dom {
    snapshot: Option<Snapshot>,
    nodes: BTreeMap<i32, Element>,
    next: i32,
    control_ids: BTreeMap<String, i32>,
    child_ids: BTreeMap<i32, Vec<i32>>,
    omitted_id: i32,
    sent: BTreeSet<i32>,
    children_sent: BTreeSet<i32>,
    style_updates: BTreeSet<i32>,
    accessibility_updates: BTreeSet<i32>,
    updates: Vec<Event>,
    searches: BTreeMap<u64, Vec<i32>>,
    next_search: u64,
    node_objects: BTreeMap<u64, (i32, Option<String>)>,
    next_node_object: u64,
}

impl Dom {
    pub fn update(&mut self, snapshot: Snapshot) -> Result<bool, Error> {
        let reset = self.snapshot.as_ref().is_some_and(|old| {
            old.status != snapshot.status
                || (old.capture != snapshot.capture
                    && !old
                        .elements
                        .iter()
                        .chain(&snapshot.elements)
                        .any(|e| e.stable_id.is_some()))
        });
        let old_children: BTreeMap<_, _> = std::iter::once(1)
            .chain(self.nodes.keys().copied())
            .map(|id| (id, self.children(id)))
            .collect();
        let expanded = self.children_sent.clone();
        let mut nodes = BTreeMap::new();
        let mut used = BTreeSet::new();

        let mut counts = BTreeMap::new();
        for element in &snapshot.elements {
            if let Some(id) = &element.stable_id {
                *counts.entry(id).or_insert(0usize) += 1;
            }
        }
        let old_ids: BTreeMap<_, _> = self.nodes.iter().map(|(&id, e)| (&e.id, (id, e))).collect();
        let mut old_stable = BTreeMap::new();
        for (&id, element) in &self.nodes {
            if let Some(stable) = &element.stable_id {
                let entry = old_stable.entry(stable).or_insert(Vec::new());
                entry.push((id, element));
            }
        }
        // Release borrows before allocating new CDP IDs.
        let existing: Vec<_> = snapshot
            .elements
            .iter()
            .map(|element| {
                let candidate = old_ids.get(&element.id).copied().or_else(|| {
                    let stable = element.stable_id.as_ref()?;
                    let old = old_stable.get(stable)?;
                    (counts.get(stable) == Some(&1) && old.len() == 1).then_some(old[0])
                });
                candidate
                    .filter(|(_, old)| {
                        !reset
                            && old.kind == element.kind
                            && old.display_name() == element.display_name()
                            && old.text.is_some() == element.text.is_some()
                            && old.tab_id == element.tab_id
                            && old.entry_id == element.entry_id
                    })
                    .map(|(id, old)| (id, old.clone()))
            })
            .collect();
        for (element, existing) in snapshot.elements.iter().zip(existing) {
            let existing = existing.filter(|(id, _)| !used.contains(id));
            let id = if let Some((id, old)) = existing {
                if old.accessibility != element.accessibility {
                    self.accessibility_updates.insert(id);
                }
                if (old.styles != element.styles
                    || old.resolved != element.resolved
                    || old.layout != element.layout
                    || old.box_model != element.box_model
                    || old.parent != element.parent)
                    && self.sent.contains(&id)
                {
                    self.style_updates.insert(id);
                }
                id
            } else {
                self.allocate()?
            };
            used.insert(id);
            nodes.insert(id, element.clone());
        }

        let old_nodes = std::mem::replace(&mut self.nodes, nodes);
        let old_snapshot = self.snapshot.replace(snapshot);
        let previous_omitted = self.omitted_id;
        if self.snapshot.as_ref().is_some_and(|s| s.omitted > 0)
            && old_snapshot.as_ref().is_none_or(|s| s.omitted == 0)
        {
            self.omitted_id = self.allocate()?;
        }

        self.control_ids = self
            .nodes
            .iter()
            .map(|(&id, e)| (e.id.clone(), id))
            .collect();
        self.child_ids.clear();
        for element in self.snapshot.iter().flat_map(|s| &s.elements) {
            let parent = element
                .parent
                .as_ref()
                .and_then(|id| self.control_ids.get(id))
                .copied()
                .unwrap_or(1);
            self.child_ids
                .entry(parent)
                .or_default()
                .push(self.control_ids[&element.id]);
        }
        if self.snapshot.as_ref().is_some_and(|s| s.omitted > 0) {
            self.child_ids.entry(1).or_default().push(self.omitted_id);
        }

        for (&parent, children) in &old_children {
            if self.children(parent) != *children {
                self.accessibility_updates.insert(parent);
            }
        }
        if reset {
            self.reset_delivery();
            return Ok(true);
        }

        // Remove only changed edges. LCS keeps unaffected sibling objects in Chrome.
        let old_ids: BTreeMap<_, _> = old_nodes
            .iter()
            .map(|(&id, element)| (&element.id, id))
            .collect();
        let parents = std::iter::once(1)
            .chain(
                old_snapshot
                    .iter()
                    .flat_map(|s| &s.elements)
                    .filter_map(|element| old_ids.get(&element.id).copied()),
            )
            .collect::<Vec<_>>();
        for parent in parents {
            if !self.sent.contains(&parent) || !self.children_sent.contains(&parent) {
                continue;
            }
            let before = &old_children[&parent];
            let after = self.children(parent);
            let retained = common_subsequence(before, &after);
            for &child in before {
                if !retained.contains(&child) && self.sent.contains(&child) {
                    self.updates.push(Event::new(
                        "DOM.childNodeRemoved",
                        json!({"parentNodeId":parent,"nodeId":child}),
                    ));
                    self.forget(child, &old_children);
                }
            }
        }

        let parents = std::iter::once(1)
            .chain(
                self.snapshot
                    .iter()
                    .flat_map(|s| &s.elements)
                    .filter_map(|element| self.node_id(&element.id)),
            )
            .collect::<Vec<_>>();
        for parent in parents {
            if !self.sent.contains(&parent) {
                continue;
            }
            let children = self.children(parent);
            if self.children_sent.contains(&parent) {
                let mut previous = 0;
                for child in children {
                    if !self.sent.contains(&child) {
                        let node = self.inserted_node(child, &expanded);
                        self.remember(&node);
                        self.updates.push(Event::new(
                            "DOM.childNodeInserted",
                            json!({"parentNodeId":parent,"previousNodeId":previous,"node":node}),
                        ));
                    }
                    previous = child;
                }
            } else if old_children.get(&parent) != Some(&children) {
                self.updates.push(Event::new(
                    "DOM.childNodeCountUpdated",
                    json!({"nodeId":parent,"childNodeCount":children.len()}),
                ));
            }
        }

        for (&id, element) in &self.nodes {
            if self.sent.contains(&id)
                && let Some(old) = old_nodes.get(&id)
                && old.text != element.text
                && let Some(text) = &element.text
            {
                self.updates.push(Event::new(
                    "DOM.characterDataModified",
                    json!({"nodeId":id,"characterData":text}),
                ));
            }
            if self.sent.contains(&id)
                && element.text.is_none()
                && let Some(old) = old_nodes.get(&id)
                && old.semantic_id != element.semantic_id
            {
                self.updates.push(match &element.semantic_id {
                    Some(value) => Event::new(
                        "DOM.attributeModified",
                        json!({"nodeId":id,"name":"id","value":value}),
                    ),
                    None => Event::new("DOM.attributeRemoved", json!({"nodeId":id,"name":"id"})),
                });
            }
        }
        if self.omitted_id == previous_omitted
            && self.sent.contains(&self.omitted_id)
            && let (Some(old), Some(current)) = (&old_snapshot, &self.snapshot)
            && old.omitted != current.omitted
            && current.omitted > 0
        {
            self.updates.push(Event::new("DOM.characterDataModified", json!({"nodeId":self.omitted_id,"characterData":format!("{} elements omitted", current.omitted)})));
        }
        self.style_updates
            .retain(|id| self.nodes.contains_key(id) && self.sent.contains(id));

        Ok(false)
    }

    fn allocate(&mut self) -> Result<i32, Error> {
        let id = self.next.max(2);
        self.next = id
            .checked_add(1)
            .ok_or_else(|| Error::new(-32000, "CDP node identity exhausted; reconnect"))?;
        Ok(id)
    }

    pub fn reset_delivery(&mut self) {
        self.sent.clear();
        self.children_sent.clear();
        self.style_updates.clear();
        self.accessibility_updates.clear();
        self.updates.clear();
    }

    pub fn take_updates(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.updates)
    }

    fn forget(&mut self, id: i32, old_children: &BTreeMap<i32, Vec<i32>>) {
        self.sent.remove(&id);
        self.children_sent.remove(&id);
        if let Some(children) = old_children.get(&id) {
            for &child in children {
                self.forget(child, old_children);
            }
        }
    }

    fn inserted_node(&self, id: i32, expanded: &BTreeSet<i32>) -> Value {
        let mut node = self.node(id, 0);
        if expanded.contains(&id) {
            node["children"] = json!(
                self.children(id)
                    .into_iter()
                    .map(|child| self.inserted_node(child, expanded))
                    .collect::<Vec<_>>()
            );
        }
        node
    }

    pub fn generation(&self) -> Option<&str> {
        self.snapshot.as_ref().map(|s| s.capture.as_str())
    }

    pub fn search(&mut self, query: &str) -> Result<Value, Error> {
        if query.trim().is_empty() || query.len() > 1024 {
            return Err(Error::new(-32602, "Search must contain 1–1024 bytes"));
        }
        let lower = query.to_lowercase();
        let results = self
            .snapshot
            .iter()
            .flat_map(|s| &s.elements)
            .filter(|element| {
                if let Some(id) = query.strip_prefix('#') {
                    element.semantic_id.as_deref() == Some(id)
                } else {
                    [
                        element.kind.as_str(),
                        element.id.as_str(),
                        element.display_name(),
                    ]
                    .into_iter()
                    .chain(element.semantic_id.iter().map(String::as_str))
                    .chain(element.stable_id.iter().map(String::as_str))
                    .chain(element.text.iter().map(String::as_str))
                    .any(|value| value.to_lowercase().contains(&lower))
                }
            })
            .filter_map(|element| self.node_id(&element.id))
            .collect::<Vec<_>>();
        self.next_search = self
            .next_search
            .checked_add(1)
            .ok_or_else(|| Error::new(-32000, "Search identity exhausted; reconnect"))?;
        if self.searches.len() == 8 {
            self.searches.pop_first();
        }
        let count = results.len();
        self.searches.insert(self.next_search, results);
        Ok(json!({"searchId":format!("search-{}",self.next_search),"resultCount":count}))
    }

    pub fn search_results(&mut self, params: &Value) -> Result<(Value, Vec<Value>), Error> {
        let search = search_id(params)?;
        let results = self
            .searches
            .get(&search)
            .ok_or_else(|| Error::new(-32000, "Search expired; repeat the search"))?;
        let from = params["fromIndex"]
            .as_u64()
            .and_then(|v| usize::try_from(v).ok());
        let to = params["toIndex"]
            .as_u64()
            .and_then(|v| usize::try_from(v).ok());
        let (Some(from), Some(to)) = (from, to) else {
            return Err(Error::new(-32602, "Search range requires integer indices"));
        };
        if from >= to || to > results.len() {
            return Err(Error::new(-32602, "Invalid search result range"));
        }
        let ids = results[from..to].to_vec();
        if ids.iter().any(|id| !self.nodes.contains_key(id)) {
            return Err(Error::new(
                -32000,
                "Search result was removed; repeat the search",
            ));
        }
        self.push_nodes(&json!({"backendNodeIds":ids}))
    }

    pub fn discard_search(&mut self, params: &Value) -> Result<Value, Error> {
        self.searches.remove(&search_id(params)?);
        Ok(json!({}))
    }

    pub fn navigation_command(
        &self,
        method: &str,
        params: &Value,
    ) -> Result<rake_protocol::control::Command, Error> {
        let element = self.element(node_id(params)?)?;
        let action = match method {
            "DOM.focus" => rake_protocol::inspection::NavigationAction::Focus,
            "DOM.scrollIntoViewIfNeeded" => {
                rake_protocol::inspection::NavigationAction::ScrollIntoView
            }
            _ => return Err(Error::new(-32601, "Unknown navigation action")),
        };
        Ok(rake_protocol::control::Command::NavigateElement {
            id: element.id.clone(),
            generation: self.generation().unwrap_or_default().into(),
            action,
        })
    }

    pub fn resolve_node(&mut self, params: &Value) -> Result<Value, Error> {
        let id = node_id(params)?;
        let element = self.element(id)?;
        let name = element.display_name().to_owned();
        let description = format!(
            "{}{}",
            name,
            element
                .semantic_id
                .as_ref()
                .map_or_else(String::new, |id| format!("#{id}"))
        );
        let group = params["objectGroup"].as_str().map(str::to_owned);
        if group.as_ref().is_some_and(|group| group.len() > 128) {
            return Err(Error::new(-32602, "Object group exceeds 128 bytes"));
        }
        self.next_node_object = self
            .next_node_object
            .checked_add(1)
            .ok_or_else(|| Error::new(-32000, "Node object identity exhausted; reconnect"))?;
        if self.node_objects.len() == 128 {
            self.node_objects.pop_first();
        }
        self.node_objects.insert(self.next_node_object, (id, group));
        Ok(
            json!({"object":{"type":"object","subtype":"node","className":name,"description":description,"objectId":format!("rake-node-{}",self.next_node_object)}}),
        )
    }

    pub fn object_node(&self, id: &str) -> Result<i32, Error> {
        let node = id
            .strip_prefix("rake-node-")
            .and_then(|id| id.parse::<u64>().ok())
            .and_then(|id| self.node_objects.get(&id))
            .map(|(node, _)| *node)
            .ok_or_else(|| Error::new(-32000, "Node object expired or unavailable"))?;
        self.element(node)?;
        Ok(node)
    }

    pub fn release_node_object(&mut self, id: &str) {
        if let Some(id) = id
            .strip_prefix("rake-node-")
            .and_then(|id| id.parse::<u64>().ok())
        {
            self.node_objects.remove(&id);
        }
    }

    pub fn release_node_group(&mut self, group: &str) {
        self.node_objects
            .retain(|_, (_, owner)| owner.as_deref() != Some(group));
    }

    pub fn call_navigation(
        &self,
        params: &Value,
    ) -> Result<rake_protocol::control::Command, Error> {
        if params["throwOnSideEffect"] == true
            || params
                .get("executionContextId")
                .is_some_and(|id| id.as_u64() != Some(1))
            || (!params["arguments"].is_null()
                && params["arguments"]
                    .as_array()
                    .is_none_or(|args| !args.is_empty()))
        {
            return Err(Error::new(
                -32602,
                "Navigation requires the current context and no arguments/side-effect restriction",
            ));
        }
        let object = params["objectId"]
            .as_str()
            .ok_or_else(|| Error::new(-32602, "objectId required"))?;
        let node = self.object_node(object)?;
        let function = params["functionDeclaration"]
            .as_str()
            .ok_or_else(|| Error::new(-32602, "functionDeclaration required"))?;
        let function = function
            .chars()
            .filter(|c| !c.is_ascii_whitespace())
            .collect::<String>();
        let body = function
            .strip_prefix("function")
            .and_then(|function| function.split_once('{'))
            .filter(|(signature, _)| {
                signature.strip_suffix("()").is_some_and(|name| {
                    name.chars().enumerate().all(|(index, character)| {
                        character.is_ascii_alphabetic()
                            || matches!(character, '_' | '$')
                            || (index > 0 && character.is_ascii_digit())
                    })
                })
            })
            .and_then(|(_, body)| body.strip_suffix('}'))
            .map(|body| body.trim_end_matches(';'));
        let method = match body {
            Some("this.focus()") => "DOM.focus",
            Some("this.scrollIntoViewIfNeeded(true)" | "this.scrollIntoViewIfNeeded(!0)") => {
                "DOM.scrollIntoViewIfNeeded"
            }
            _ => {
                return Err(Error::new(
                    -32601,
                    "Only Chrome's focus/scroll navigation helpers are supported; JavaScript is never evaluated",
                ));
            }
        };
        self.navigation_command(method, &json!({"nodeId":node}))
    }

    pub fn node_properties(&self, object: &str) -> Result<Value, Error> {
        let element = self.element(self.object_node(object)?)?;
        let mut result = vec![
            json!({"name":"nodeName","value":{"type":"string","value":if element.text.is_some() { "#text" } else { element.display_name() }},"configurable":false,"enumerable":true}),
            json!({"name":"id","value":{"type":"string","value":element.semantic_id.as_deref().unwrap_or_default()},"configurable":false,"enumerable":true}),
        ];
        if let Some(text) = &element.text {
            result.push(json!({"name":"textContent","value":{"type":"string","value":text},"configurable":false,"enumerable":true}));
        }
        if let Some(view) = &element.view {
            result.push(json!({"name":"rakeStyleSources","value":{"type":"string","value":serde_json::to_string_pretty(&view.style_sources).unwrap_or_default()},"configurable":false,"enumerable":true}));
        }
        if let Some(a11y) = &element.accessibility {
            result.push(json!({"name":"rakeAccessibility","value":{"type":"string","value":serde_json::to_string_pretty(a11y).unwrap_or_default()},"configurable":false,"enumerable":true}));
        }
        result.push(json!({"name":"rakeHandlers","value":{"type":"string","value":serde_json::to_string_pretty(&element.handlers).unwrap_or_default()},"configurable":false,"enumerable":true}));
        Ok(json!({"result":result}))
    }

    pub fn source(
        &self,
        selector: &Value,
    ) -> Result<&rake_protocol::inspection::SourceLocation, Error> {
        let target = selector
            .get("target")
            .cloned()
            .map(serde_json::from_value::<rake_protocol::inspection::SourceTarget>)
            .transpose()
            .map_err(|_| Error::new(-32602, "Invalid captured source target"))?
            .unwrap_or_default();
        let selector = selector.get("nodeId").unwrap_or(selector);
        let element = if let Some(id) = selector.as_i64().and_then(|id| i32::try_from(id).ok()) {
            self.element(id)?
        } else if let Some(name) = selector.as_str() {
            let mut matches = self.nodes.values().filter(|element| {
                element.semantic_id.as_deref() == Some(name) || element.id == name
            });
            let element = matches
                .next()
                .ok_or_else(|| Error::new(-32000, "Source target not found"))?;
            if matches.next().is_some() {
                return Err(Error::new(
                    -32602,
                    "Source target is ambiguous; use a node ID",
                ));
            }
            element
        } else {
            return Err(Error::new(
                -32602,
                "Expected a node ID or exact semantic ID",
            ));
        };
        element
            .source_location(&target)
            .ok_or_else(|| Error::new(-32000, "Captured source location unavailable"))
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
        self.control_ids.get(id).copied()
    }

    pub fn overlay_command(
        &self,
        method: &str,
        params: &Value,
    ) -> Result<rake_protocol::control::Command, Error> {
        use rake_protocol::control::Command;

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
        self.control_ids.clear();
        self.child_ids.clear();
        self.sent.clear();
        self.children_sent.clear();
        self.style_updates.clear();
        self.accessibility_updates.clear();
        self.updates.clear();
        self.searches.clear();
        self.node_objects.clear();
    }

    fn element(&self, id: i32) -> Result<&Element, Error> {
        self.nodes
            .get(&id)
            .ok_or_else(|| Error::new(-32000, "Node is stale or unavailable; refresh the document"))
    }

    fn children(&self, id: i32) -> Vec<i32> {
        self.child_ids.get(&id).cloned().unwrap_or_default()
    }

    fn ax_node(&self, id: i32) -> Value {
        let element = self.nodes.get(&id);
        let parent = element.map(|element| {
            element
                .parent
                .as_deref()
                .and_then(|id| self.node_id(id))
                .unwrap_or(1)
        });
        super::accessibility::node(
            id,
            element,
            parent,
            self.children(id)
                .into_iter()
                .filter(|child| self.nodes.contains_key(child))
                .collect(),
        )
    }

    pub fn take_accessibility_updates(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.accessibility_updates)
            .into_iter()
            .filter(|id| *id == 1 || self.nodes.contains_key(id))
            .map(|id| self.ax_node(id))
            .collect()
    }

    pub fn accessibility(&self, method: &str, params: &Value) -> Result<Value, Error> {
        if self
            .snapshot
            .as_ref()
            .is_none_or(|snapshot| snapshot.status != rake_protocol::inspection::Status::Captured)
        {
            return Err(Error::new(
                -32000,
                "Accessibility is waiting for a captured inspection host",
            ));
        }
        let target = || -> Result<i32, Error> {
            if let Some(id) = params["objectId"].as_str() {
                self.object_node(id)
            } else {
                node_id(params)
            }
        };
        if method == "Accessibility.getRootAXNode" {
            return Ok(json!({"node":self.ax_node(1)}));
        }
        let mut ids = BTreeSet::new();
        match method {
            "Accessibility.getFullAXTree" => {
                let depth = depth(params, 128)?.min(128);
                let mut pending = vec![(1, 0)];
                while let Some((id, level)) = pending.pop() {
                    if !ids.insert(id) {
                        continue;
                    }
                    if level < depth {
                        pending.extend(
                            self.children(id)
                                .into_iter()
                                .filter(|child| self.nodes.contains_key(child))
                                .map(|child| (child, level + 1)),
                        );
                    }
                }
            }
            "Accessibility.getChildAXNodes" => {
                let id = params["id"]
                    .as_str()
                    .and_then(|id| id.strip_prefix("ax-"))
                    .and_then(|id| id.parse().ok())
                    .ok_or_else(|| Error::new(-32602, "AX node id required"))?;
                if id != 1 {
                    self.element(id)?;
                }
                ids.extend(
                    self.children(id)
                        .into_iter()
                        .filter(|id| self.nodes.contains_key(id)),
                );
            }
            "Accessibility.getPartialAXTree" | "Accessibility.getAXNodeAndAncestors" => {
                let target = target()?;
                if target != 1 {
                    self.element(target)?;
                }
                ids.insert(target);
                if params["fetchRelatives"] != false {
                    let mut id = target;
                    while let Some(parent) = self.nodes.get(&id).map(|e| {
                        e.parent
                            .as_deref()
                            .and_then(|id| self.node_id(id))
                            .unwrap_or(1)
                    }) {
                        if !ids.insert(parent) {
                            break;
                        }
                        id = parent;
                    }
                    if method == "Accessibility.getPartialAXTree" {
                        ids.extend(
                            self.children(target)
                                .into_iter()
                                .filter(|id| self.nodes.contains_key(id)),
                        );
                    }
                }
            }
            _ => return Err(Error::new(-32601, "Unsupported accessibility method")),
        }
        let value = json!({"nodes":ids.into_iter().map(|id| self.ax_node(id)).collect::<Vec<_>>()});
        if serde_json::to_vec(&value).map_or(usize::MAX, |bytes| bytes.len()) > 512 * 1024 {
            return Err(Error::new(
                -32000,
                "Accessibility response exceeds budget; request the root and individual children",
            ));
        }
        Ok(value)
    }

    pub fn edit_style(&self, id: i32) -> Result<Value, Error> {
        Ok(super::editing::style(id, self.element(id)?))
    }
    pub fn edit_text(&self, id: i32) -> Result<String, Error> {
        Ok(super::editing::text(self.element(id)?))
    }
    pub fn editable(&self, id: i32) -> bool {
        self.element(id).is_ok_and(|element| element.style_editable)
    }
    pub fn edit_command(
        &self,
        id: i32,
        edit: rake_protocol::inspection::StyleEdit,
    ) -> Result<rake_protocol::control::Command, Error> {
        let element = self.element(id)?;
        if !element.style_editable {
            return Err(Error::new(-32000, "Boundary is not style editable"));
        }
        Ok(rake_protocol::control::Command::EditElementStyle {
            id: element.id.clone(),
            generation: self.generation().unwrap_or_default().into(),
            edit,
        })
    }

    pub fn control_id(&self, id: i32) -> Result<&str, Error> {
        Ok(&self.element(id)?.id)
    }

    pub fn details(&mut self, id: i32, element: Element) -> Result<(), Error> {
        if self.element(id)?.id != element.id {
            return Err(Error::new(-32000, "Inspection capture changed; retry"));
        }
        self.nodes.insert(id, element);
        Ok(())
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
            let name = element.display_name();

            if let Some(text) = &element.text {
                json!({"nodeId":id,"backendNodeId":id,"nodeType":3,"nodeName":"#text","localName":"","nodeValue":text})
            } else {
                json!({"nodeId":id,"backendNodeId":id,"nodeType":1,"nodeName":name,"localName":name,
                    "nodeValue":"","attributes":attributes(element)})
            }
        };

        node["childNodeCount"] = json!(children.len());

        // Chrome inlines a sole text child only when it arrives with its parent.
        let inline_text = children.len() == 1
            && self
                .nodes
                .get(&children[0])
                .is_some_and(|element| element.text.is_some());
        if depth > 0 || children.is_empty() || inline_text {
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
                let model = element.box_model.as_ref().ok_or_else(|| {
                    Error::new(
                        -32000,
                        "Resolved box geometry unavailable for this node/runtime",
                    )
                })?;
                let (Some(content), Some(padding), Some(margin)) =
                    (model.content, model.padding, model.margin)
                else {
                    return Err(Error::new(
                        -32000,
                        format!("Incomplete box geometry: {}", model.unavailable.join("; ")),
                    ));
                };
                let quad = |layout: rake_protocol::inspection::Layout| {
                    let (x, y, w, h) = (layout.x, layout.y, layout.width, layout.height);
                    json!([x, y, x + w, y, x + w, y + h, x, y + h])
                };
                Ok(
                    json!({"model":{"content":quad(content),"padding":quad(padding),
                    "border":quad(model.border),"margin":quad(margin),
                    "width":model.border.width.round() as i32,"height":model.border.height.round() as i32}}),
                )
            }
            "CSS.getInlineStylesForNode" => {
                Ok(json!({"inlineStyle":super::styles::inline(element.styles.as_ref())}))
            }
            "CSS.getMatchedStylesForNode" => {
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

                let mut rules = super::styles::matched(element);
                if element.style_editable {
                    rules.push(super::editing::rule(id, element));
                }
                if rules.is_empty() {
                    Ok(
                        json!({"inlineStyle":super::styles::inline(element.styles.as_ref()),"matchedCSSRules":[],"inherited":inherited,"pseudoElements":[]}),
                    )
                } else {
                    Ok(json!({"matchedCSSRules":rules,"inherited":inherited,"pseudoElements":[]}))
                }
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

// Sibling IDs are unique: LCS is the longest increasing subsequence of old positions.
fn common_subsequence(before: &[i32], after: &[i32]) -> BTreeSet<i32> {
    let positions: BTreeMap<_, _> = before.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let sequence: Vec<_> = after
        .iter()
        .filter_map(|id| positions.get(id).map(|&p| (*id, p)))
        .collect();
    let mut tails: Vec<usize> = Vec::new();
    let mut previous = vec![None; sequence.len()];
    for (i, &(_, position)) in sequence.iter().enumerate() {
        let j = tails.partition_point(|&index| sequence[index].1 < position);
        previous[i] = j.checked_sub(1).map(|j| tails[j]);
        if j == tails.len() {
            tails.push(i);
        } else {
            tails[j] = i;
        }
    }
    let mut common = BTreeSet::new();
    let mut cursor = tails.last().copied();
    while let Some(i) = cursor {
        common.insert(sequence[i].0);
        cursor = previous[i];
    }
    common
}

fn visual_style(element: &Element) -> Option<&Value> {
    element
        .resolved
        .as_ref()
        .map(|resolved| &resolved.properties)
        .or(element.styles.as_ref())
}

fn search_id(params: &Value) -> Result<u64, Error> {
    params["searchId"]
        .as_str()
        .and_then(|id| id.strip_prefix("search-"))
        .and_then(|id| id.parse().ok())
        .ok_or_else(|| Error::new(-32602, "Invalid searchId"))
}

fn attributes(element: &Element) -> Vec<String> {
    let mut attrs = Vec::new();

    if element.text.is_some() {
        return attrs;
    }

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
        -1 => Ok(rake_protocol::inspection::MAX_ELEMENTS + 1),
        0.. => Ok((depth as usize).min(rake_protocol::inspection::MAX_ELEMENTS + 1)),
        _ => Err(Error::new(-32602, "depth must be -1 or nonnegative")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stable_capture(generation: &str, tree: &[(&str, Option<&str>)]) -> Snapshot {
        serde_json::from_value(json!({"status":"captured","capture":generation,"router_revision":0,"omitted":0,"coverage":"test",
            "elements":tree.iter().map(|(name,parent)| json!({
                "id":format!("{generation}-{name}"),"stable_id":name,
                "parent":parent.map(|name| format!("{generation}-{name}")),"kind":"Div",
                "semantic_id":name,"layout":{"x":0.,"y":0.,"width":10.,"height":10.},"metadata_truncated":false
            })).collect::<Vec<_>>()
        })).unwrap()
    }

    #[test]
    fn accessibility_projection_exposes_real_labels_and_updates_without_losing_dom_identity() {
        let mut capture = stable_capture("1", &[("root", None), ("button", Some("root"))]);
        capture.elements[1].accessibility =
            Some(Box::new(rake_protocol::inspection::Accessibility {
                role: Some("Button".into()),
                label: Some("Save".into()),
                name: None,
                name_source: None,
                states: Default::default(),
                description: None,
                disabled: false,
                hidden: false,
                focusable: true,
                focused: Some(false),
                contains_focus: Some(false),
                tab_index: Some(0),
                tab_stop: Some(true),
                captured_tab_order: Some(1),
                actions: vec!["Click".into(), "Focus".into()],
            }));
        let mut dom = Dom::default();
        dom.update(capture.clone()).unwrap();
        let button = dom.node_id("1-button").unwrap();
        let tree = dom
            .accessibility(
                "Accessibility.getPartialAXTree",
                &json!({"backendNodeId":button}),
            )
            .unwrap();
        let node = tree["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["backendDOMNodeId"] == button)
            .unwrap();
        assert_eq!(node["role"]["value"], "button");
        assert_eq!(node["name"]["value"], "Save");
        assert_eq!(node["ignored"], false);
        let root = dom
            .accessibility("Accessibility.getRootAXNode", &json!({}))
            .unwrap();
        assert_eq!(root["node"]["role"]["value"], "application");
        dom.take_accessibility_updates();
        capture.elements[1].accessibility.as_mut().unwrap().focused = Some(true);
        let a11y = capture.elements[1].accessibility.as_mut().unwrap();
        a11y.name = Some("Save document".into());
        a11y.name_source = Some("contents".into());
        a11y.states.insert("pressed".into(), "mixed".into());
        a11y.states.insert("expanded".into(), false.into());
        dom.update(capture).unwrap();
        assert_eq!(dom.node_id("1-button"), Some(button));
        let tree = dom
            .accessibility(
                "Accessibility.getPartialAXTree",
                &json!({"backendNodeId":button}),
            )
            .unwrap();
        let node = tree["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["backendDOMNodeId"] == button)
            .unwrap();
        assert_eq!(node["name"]["value"], "Save document");
        assert!(
            node["properties"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["name"] == "pressed"
                    && p["value"]["type"] == "tristate"
                    && p["value"]["value"] == "mixed")
        );
        assert_eq!(
            dom.take_accessibility_updates()[0]["nodeId"],
            format!("ax-{button}")
        );
        assert!(
            dom.accessibility("Accessibility.getChildAXNodes", &json!({"id":"stale"}))
                .is_err()
        );
    }

    #[test]
    fn large_sibling_lists_load_lazily_and_keep_identity_on_reorder() {
        let names: Vec<_> = (0..4000).map(|i| format!("child-{i}")).collect();
        let mut tree = vec![("root", None)];
        tree.extend(names.iter().map(|name| (name.as_str(), Some("root"))));
        let mut dom = Dom::default();
        dom.update(stable_capture("1", &tree)).unwrap();
        let document = dom.document(&json!({"depth":1})).unwrap();
        assert!(document["children"][0].get("children").is_none());
        let id = dom.node_id("1-child-3999").unwrap();
        tree[1..].reverse();
        assert!(!dom.update(stable_capture("2", &tree)).unwrap());
        assert_eq!(dom.node_id("2-child-3999"), Some(id));
        assert_eq!(dom.search("#child-3999").unwrap()["resultCount"], 1);
    }

    #[test]
    fn text_nodes_show_content_and_update_without_replacing_the_selected_node() {
        let mut capture = stable_capture("1", &[("p", None), ("text", Some("p"))]);
        capture.elements[1].text = Some("Hello <script>世 🌍</script>\n".into());
        let mut dom = Dom::default();
        dom.update(capture.clone()).unwrap();
        let preview = dom.node(dom.node_id("1-p").unwrap(), 0);
        assert_eq!(
            preview["children"][0]["nodeType"], 3,
            "lazy parent previews include their sole text child"
        );
        let document = dom.document(&json!({"depth":-1})).unwrap();
        let text = &document["children"][0]["children"][0];
        assert_eq!(text["nodeType"], 3);
        assert_eq!(text["nodeName"], "#text");
        assert_eq!(
            text["nodeValue"],
            capture.elements[1].text.as_ref().unwrap().as_str()
        );
        assert!(text.get("attributes").is_none());
        let id = dom.node_id("1-text").unwrap();
        assert_eq!(dom.search("世 🌍").unwrap()["resultCount"], 1);

        capture = stable_capture("2", &[("p", None), ("text", Some("p"))]);
        capture.elements[1].text = Some(String::new());
        assert!(!dom.update(capture.clone()).unwrap());
        assert_eq!(dom.node_id("2-text"), Some(id));
        let events = dom.take_updates();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].method, "DOM.characterDataModified");
        assert_eq!(events[0].params, json!({"nodeId":id,"characterData":""}));

        capture.elements[1].text = None;
        assert!(!dom.update(capture).unwrap());
        assert_ne!(
            dom.node_id("2-text"),
            Some(id),
            "node type changes replace the node"
        );
    }

    #[test]
    fn geometry_and_metadata_updates_keep_nodes_and_expansion_across_captures() {
        let tree = [("root", None), ("child", Some("root"))];
        let mut dom = Dom::default();
        dom.update(stable_capture("1", &tree)).unwrap();
        dom.document(&json!({"depth":-1})).unwrap();
        let root = dom.node_id("1-root").unwrap();
        let child = dom.node_id("1-child").unwrap();
        let mut changed = stable_capture("2", &tree);
        changed.elements[1].layout.as_mut().unwrap().width = 42.;
        changed.elements[1].semantic_id = Some("renamed".into());

        assert!(!dom.update(changed).unwrap());
        assert_eq!(dom.node_id("2-root"), Some(root));
        assert_eq!(dom.node_id("2-child"), Some(child));
        assert!(
            dom.request_children(&json!({"nodeId":root,"depth":-1}))
                .unwrap()
                .is_empty()
        );
        let events = dom.take_updates();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].method, "DOM.attributeModified");
        assert_eq!(
            events[0].params,
            json!({"nodeId":child,"name":"id","value":"renamed"})
        );
        assert_eq!(
            dom.handle("CSS.getComputedStyleForNode", &json!({"nodeId":child}))
                .unwrap()["computedStyle"][0]["value"],
            "42px"
        );
        assert!(dom.take_style_updates().contains(&child));
        assert_eq!(
            dom.overlay_command("Overlay.highlightNode", &json!({"nodeId":child}))
                .unwrap(),
            rake_protocol::control::Command::HighlightElement {
                id: Some("2-child".into()),
                generation: "2".into()
            }
        );
    }

    #[test]
    fn structural_events_replay_to_the_new_order_without_replacing_unchanged_subtrees() {
        let mut dom = Dom::default();
        let before = [
            ("root", None),
            ("a", Some("root")),
            ("b", Some("root")),
            ("leaf", Some("b")),
            ("c", Some("root")),
        ];
        dom.update(stable_capture("1", &before)).unwrap();
        dom.document(&json!({"depth":-1})).unwrap();
        let root = dom.node_id("1-root").unwrap();
        let b = dom.node_id("1-b").unwrap();
        let leaf = dom.node_id("1-leaf").unwrap();
        let mut frontend: BTreeMap<_, _> = std::iter::once(1)
            .chain(dom.nodes.keys().copied())
            .map(|id| (id, dom.children(id)))
            .collect();

        let after = [
            ("root", None),
            ("c", Some("root")),
            ("b", Some("root")),
            ("leaf", Some("b")),
            ("new", Some("root")),
        ];
        assert!(!dom.update(stable_capture("2", &after)).unwrap());
        let events = dom.take_updates();
        for event in &events {
            let parent = event.params["parentNodeId"].as_i64().unwrap() as i32;
            let siblings = frontend.get_mut(&parent).unwrap();
            match event.method.as_str() {
                "DOM.childNodeRemoved" => {
                    let id = event.params["nodeId"].as_i64().unwrap() as i32;
                    let index = siblings.iter().position(|child| *child == id).unwrap();
                    siblings.remove(index);
                    assert_ne!(id, b, "unchanged selected subtree stays attached");
                }
                "DOM.childNodeInserted" => {
                    let previous = event.params["previousNodeId"].as_i64().unwrap() as i32;
                    let index = if previous == 0 {
                        0
                    } else {
                        siblings
                            .iter()
                            .position(|child| *child == previous)
                            .unwrap()
                            + 1
                    };
                    let id = event.params["node"]["nodeId"].as_i64().unwrap() as i32;
                    assert!(!siblings.contains(&id));
                    siblings.insert(index, id);
                }
                _ => panic!("unexpected mutation: {}", event.method),
            }
        }
        assert_eq!(frontend[&root], dom.children(root));
        assert_eq!(dom.node_id("2-b"), Some(b));
        assert_eq!(dom.node_id("2-leaf"), Some(leaf));
        assert_eq!(frontend[&b], vec![leaf]);
        assert!(
            dom.request_children(&json!({"nodeId":b}))
                .unwrap()
                .is_empty()
        );
        assert!(dom.node_id("1-a").is_none());
    }

    #[test]
    fn collapsed_branches_receive_counts_then_expand_the_current_capture() {
        let mut dom = Dom::default();
        dom.update(stable_capture("1", &[("root", None), ("a", Some("root"))]))
            .unwrap();
        dom.document(&json!({})).unwrap();
        let root = dom.node_id("1-root").unwrap();
        dom.update(stable_capture(
            "2",
            &[("root", None), ("a", Some("root")), ("b", Some("root"))],
        ))
        .unwrap();
        let events = dom.take_updates();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].method, "DOM.childNodeCountUpdated");
        assert_eq!(events[0].params, json!({"nodeId":root,"childNodeCount":2}));
        let children = dom.request_children(&json!({"nodeId":root})).unwrap();
        assert_eq!(children[0]["nodes"].as_array().unwrap().len(), 2);
        assert!(dom.take_updates().is_empty());
    }

    #[test]
    fn removed_boundaries_are_never_resurrected_and_runtime_reset_invalidates_every_node() {
        let mut dom = Dom::default();
        dom.update(stable_capture(
            "1",
            &[("root", None), ("child", Some("root"))],
        ))
        .unwrap();
        dom.document(&json!({"depth":-1})).unwrap();
        let child = dom.node_id("1-child").unwrap();
        dom.update(stable_capture("2", &[("root", None)])).unwrap();
        assert!(dom.element(child).is_err());
        dom.update(stable_capture(
            "3",
            &[("root", None), ("child", Some("root"))],
        ))
        .unwrap();
        assert_ne!(dom.node_id("3-child"), Some(child));
        let root = dom.node_id("3-root").unwrap();
        dom.clear();
        dom.update(stable_capture("1", &[("root", None)])).unwrap();
        assert_ne!(dom.node_id("1-root"), Some(root));
        assert!(dom.element(root).is_err());
    }

    #[test]
    fn name_and_id_search_reveals_collapsed_ancestors_and_rejects_expired_results() {
        let mut dom = Dom::default();
        let tree = [
            ("root", None),
            ("panel", Some("root")),
            ("save", Some("panel")),
        ];
        dom.update(stable_capture("1", &tree)).unwrap();
        dom.document(&json!({})).unwrap();
        let save = dom.node_id("1-save").unwrap();
        let search = dom.search("#save").unwrap();
        assert_eq!(search["resultCount"], 1);
        let params = json!({"searchId":search["searchId"],"fromIndex":0,"toIndex":1});
        let (result, paths) = dom.search_results(&params).unwrap();
        assert_eq!(result["nodeIds"], json!([save]));
        assert_eq!(paths.len(), 2);
        assert!(
            dom.search_results(&params).unwrap().1.is_empty(),
            "search reveal must preserve existing nodes"
        );
        assert_eq!(dom.search("dIv").unwrap()["resultCount"], 3);
        assert_eq!(dom.search("SAV").unwrap()["resultCount"], 1);
        assert!(
            dom.search_results(&json!({"searchId":search["searchId"],"fromIndex":1,"toIndex":0}))
                .is_err()
        );
        dom.update(stable_capture("2", &tree[..2])).unwrap();
        assert!(dom.search_results(&params).is_err());
        dom.discard_search(&params).unwrap();
        assert!(dom.search_results(&params).is_err());
        assert!(dom.search("").is_err());
        assert!(dom.search(&"x".repeat(1025)).is_err());
        for _ in 0..20 {
            dom.search("root").unwrap();
        }
        assert_eq!(dom.searches.len(), 8);
        dom.clear();
        assert!(dom.searches.is_empty());
    }

    #[test]
    fn chrome_node_helpers_only_dispatch_navigation_and_respect_object_lifetimes() {
        use rake_protocol::{control::Command, inspection::NavigationAction};
        let mut dom = Dom::default();
        dom.update(stable_capture("1", &[("root", None)])).unwrap();
        let node = dom.node_id("1-root").unwrap();
        let object = dom
            .resolve_node(&json!({"nodeId":node,"objectGroup":"navigation"}))
            .unwrap();
        let id = object["object"]["objectId"].as_str().unwrap();
        for function in [
            "function focusInPage() { this.focus(); }",
            "function f(){this.focus()}",
        ] {
            assert_eq!(
                dom.call_navigation(&json!({"objectId":id,"functionDeclaration":function}))
                    .unwrap(),
                Command::NavigateElement {
                    id: "1-root".into(),
                    generation: "1".into(),
                    action: NavigationAction::Focus,
                }
            );
        }
        let scroll = json!({"objectId":id,"functionDeclaration":"function s(){this.scrollIntoViewIfNeeded(!0)}"});
        assert!(matches!(
            dom.call_navigation(&scroll).unwrap(),
            Command::NavigateElement {
                action: NavigationAction::ScrollIntoView,
                ..
            }
        ));
        for function in [
            "function(){this.focus();alert(1)}",
            "function(){return this.focus()}",
            "()=>this.focus()",
        ] {
            assert!(
                dom.call_navigation(&json!({"objectId":id,"functionDeclaration":function}))
                    .is_err()
            );
        }
        dom.update(stable_capture("2", &[("root", None)])).unwrap();
        assert!(
            matches!(dom.call_navigation(&scroll).unwrap(), Command::NavigateElement { generation, .. } if generation == "2")
        );
        dom.release_node_group("navigation");
        assert!(dom.call_navigation(&scroll).is_err());
        let object = dom.resolve_node(&json!({"nodeId":node})).unwrap();
        let id = object["object"]["objectId"].as_str().unwrap();
        dom.release_node_object(id);
        assert!(dom.object_node(id).is_err());
    }

    fn capture(id: &str) -> Snapshot {
        serde_json::from_value(json!({"status":"captured","capture":id,"router_revision":1,"omitted":2,"coverage":"test","elements":[
            {"id":format!("{id}-a"),"parent":null,"kind":"gpui::Stateful<gpui::elements::div::Div>","semantic_id":"root","source":{"file":"app.rs","line":3,"column":4},"layout":{"x":1.,"y":2.,"width":10.,"height":20.},"styles":{"display":"Flex","size":{"width":"100%"},"background":{"Color":"opaque"}},"metadata_truncated":false,"tab_id":null,"entry_id":null},
            {"id":format!("{id}-b"),"parent":format!("{id}-a"),"kind":"gpui::ViewElement<app::Button<app::Action>>","metadata_truncated":false}
        ]})).unwrap()
    }

    #[test]
    fn authored_tags_are_searchable_and_style_sources_are_available_without_preview_attributes() {
        let mut snapshot = capture("view");
        snapshot.elements[0].view = Some(
            serde_json::from_value(json!({"tag":"section", "style_sources":[{
                "scope":"section","attribute":"class","expression":"\"p-2 hover:opacity-50\"",
                "source":{"file":"src/view.rs","line":12,"column":7}
            }]}))
            .unwrap(),
        );
        let mut dom = Dom::default();
        dom.update(snapshot).unwrap();
        let document = dom.document(&json!({"depth":-1})).unwrap();
        let root = &document["children"][0];
        assert_eq!(root["nodeName"], "section");
        assert_eq!(root["attributes"], json!(["id", "root"]));
        let search = dom.search("section").unwrap();
        assert_eq!(search["resultCount"], 1);
        let object = dom.resolve_node(&json!({"nodeId":root["nodeId"]})).unwrap();
        let properties = dom
            .node_properties(object["object"]["objectId"].as_str().unwrap())
            .unwrap();
        let sources = properties["result"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "rakeStyleSources")
            .unwrap();
        let sources: Value =
            serde_json::from_str(sources["value"]["value"].as_str().unwrap()).unwrap();
        assert_eq!(sources[0]["source"]["line"], 12);
        assert_eq!(sources[0]["attribute"], "class");
        let id = root["nodeId"].as_i64().unwrap();
        assert_eq!(
            dom.source(&json!({"nodeId":id,"target":{"kind":"style","index":0}}))
                .unwrap()
                .line,
            12
        );
        assert_eq!(dom.source(&json!("root")).unwrap().line, 3);
        assert!(
            dom.source(&json!({"nodeId":id,"target":{"kind":"style","index":1}}))
                .is_err()
        );
        assert!(
            dom.source(&json!({"nodeId":id,"target":{"kind":"handler","index":0}}))
                .is_err()
        );
        assert!(
            dom.source(&json!({"nodeId":id,"target":{"kind":"boundary","file":"/secret.rs"}}))
                .is_err()
        );
    }

    #[test]
    fn box_quads_and_style_origins_preserve_distinct_geometry_and_authored_values() {
        use rake_protocol::inspection::{BoxModel, Layout, ResolvedStyle, StyleLayer, StyleOrigin};
        let mut snapshot = capture("boxes");
        let element = &mut snapshot.elements[0];
        let border = element.layout.unwrap();
        element.box_model = Some(BoxModel {
            border,
            padding: Some(Layout {
                x: 2.,
                y: 3.,
                width: 8.,
                height: 18.,
            }),
            content: Some(Layout {
                x: 3.,
                y: 4.,
                width: 6.,
                height: 16.,
            }),
            margin: Some(Layout {
                x: 0.,
                y: 1.,
                width: 12.,
                height: 22.,
            }),
            unavailable: Vec::new(),
        });
        element.styles = Some(json!({"opacity":0.5}));
        element.resolved = Some(ResolvedStyle {
            properties: json!({"opacity":0.8}),
            hovered: true,
            layers: vec![StyleLayer {
                origin: StyleOrigin::Interaction,
                properties: json!({"opacity":0.8}),
            }],
        });
        let mut dom = Dom::default();
        dom.update(snapshot.clone()).unwrap();
        let id = dom.node_id("boxes-a").unwrap();
        let params = json!({"nodeId":id});
        let model = dom.handle("DOM.getBoxModel", &params).unwrap()["model"].clone();
        assert_eq!(model["content"], json!([3., 4., 9., 4., 9., 20., 3., 20.]));
        assert_eq!(
            model["padding"],
            json!([2., 3., 10., 3., 10., 21., 2., 21.])
        );
        assert_eq!(model["border"], json!([1., 2., 11., 2., 11., 22., 1., 22.]));
        assert_eq!(model["margin"], json!([0., 1., 12., 1., 12., 23., 0., 23.]));
        let styles = dom.handle("CSS.getMatchedStylesForNode", &params).unwrap();
        let rules = styles["matchedCSSRules"].as_array().unwrap();
        assert!(
            styles.get("inlineStyle").is_none(),
            "resolved values must not masquerade as inline declarations"
        );
        assert_eq!(
            rules[0]["rule"]["style"]["cssProperties"][0]["value"],
            "0.5"
        );
        assert_eq!(
            rules[1]["rule"]["style"]["cssProperties"][0]["value"],
            "0.8"
        );
        assert!(
            rules[1]["rule"]["selectorList"]["text"]
                .as_str()
                .unwrap()
                .contains("Active interaction")
        );

        snapshot.elements[0].box_model.as_mut().unwrap().margin = None;
        dom.update(snapshot).unwrap();
        assert!(dom.handle("DOM.getBoxModel", &params).is_err());
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
            dom.handle("DOM.getBoxModel", &params).unwrap_err().code,
            -32000
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

        snapshot.elements[0].resolved = Some(rake_protocol::inspection::ResolvedStyle {
            properties: json!({"background":{"Color":{"tag":"Solid","solid":{"h":0.,"s":1.,"l":0.5,"a":1.}}},"text":{"font_size":"24px"}}),
            hovered: false,
            layers: Vec::new(),
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
        use rake_protocol::control::Command;
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
