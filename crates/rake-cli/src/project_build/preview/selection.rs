//! Exact-capture element selection, bounded annotation lookup and copied source context.
use super::{Error, Preview, Result, fields};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

impl Preview {
    pub(super) fn select(
        &self,
        action: &str,
        runtime: &str,
        window: &Value,
        point: Value,
    ) -> Result<Value> {
        let mut element = self.request(
            runtime,
            json!({"type":"preview_element","point":point}),
            false,
        )?;
        if action == "preview_select" {
            return Ok(element);
        }
        let mut seen = HashSet::new();
        for _ in 0..4 {
            let Some(parent) = element["parent"]
                .as_str()
                .filter(|parent| !parent.is_empty())
                .map(str::to_owned)
            else {
                break;
            };
            if element["view"]["tag"] != "Text" || !seen.insert(parent.clone()) {
                break;
            }
            let Ok(ancestor) = self.request(
                runtime,
                json!({"type":"inspect_element","id":parent}),
                false,
            ) else {
                break;
            };
            if ancestor["id"] != parent {
                break;
            }
            element = ancestor;
        }
        let mut selected = fields(
            &element,
            &[
                "id",
                "stable_id",
                "parent",
                "kind",
                "semantic_id",
                "view",
                "source",
                "text",
                "layout",
                "content_mask",
                "metadata_truncated",
            ],
        );
        if action == "preview_grab" {
            selected["grab"] = self.context(runtime, &window["generation"], &element);
        }
        Ok(json!({"generation":window["generation"],"element":selected}))
    }

    pub(super) fn resolve(&self, body: &Value) -> Result<Value> {
        let targets = body["targets"]
            .as_array()
            .filter(|targets| !targets.is_empty() && targets.len() <= 33)
            .ok_or_else(|| Error::new(400, "Expected up to 33 annotation targets"))?;
        for target in targets {
            let object = target
                .as_object()
                .ok_or_else(|| Error::new(400, "Invalid annotation target"))?;
            let keys: &[&str] = if target["stable_id"]
                .as_str()
                .is_some_and(|id| !id.is_empty())
            {
                &["stable_id"]
            } else {
                &["id", "generation"]
            };
            if object.len() != keys.len()
                || keys.iter().any(|key| {
                    !target[*key]
                        .as_str()
                        .is_some_and(|value| !value.is_empty() && value.chars().count() <= 128)
                })
            {
                return Err(Error::new(400, "Invalid annotation identity"));
            }
        }
        let saved = self.reference(body)?;
        let runtime = &saved.runtime;
        let window = &saved.window;
        let mut elements = vec![Value::Null; targets.len()];
        let mut page = self.request(
            runtime,
            json!({"type":"inspect_tree_page","generation":null,"offset":0}),
            false,
        )?;
        let generation = page["snapshot"]["capture"].clone();
        let windows = self.request(runtime, json!({"type":"preview"}), false)?;
        if !windows.as_array().is_some_and(|windows| {
            windows.iter().any(|w| {
                w["window"] == window["window"]
                    && w["native_id"] == window["native_id"]
                    && w["generation"] == generation
            })
        }) {
            return Err(Error::new(409, "Annotation frame changed"));
        }
        let mut offset = 0;
        let deadline = Instant::now() + Duration::from_millis(1500);
        for _ in 0..32 {
            if page["snapshot"]["capture"] != generation {
                return Err(Error::new(409, "Annotation capture changed"));
            }
            let boundaries = page["snapshot"]["elements"]
                .as_array()
                .ok_or_else(|| Error::new(409, "Invalid annotation page"))?;
            // Last matching identity wins, as in the Python dictionary indexes.
            for (index, target) in targets.iter().enumerate() {
                let element = boundaries.iter().rev().find(|element| {
                    if target.get("stable_id").is_some() {
                        element["stable_id"] == target["stable_id"]
                    } else {
                        generation == target["generation"] && element["id"] == target["id"]
                    }
                });
                if let Some(element) = element {
                    let mut selected =
                        fields(element, &["id", "stable_id", "layout", "content_mask"]);
                    if element["accessibility"]["hidden"] == true {
                        selected["layout"] = Value::Null;
                    }
                    elements[index] = selected;
                }
            }
            let next = &page["next"];
            if elements.iter().all(|element| !element.is_null()) || next.is_null() {
                return Ok(json!({"generation":generation,"elements":elements}));
            }
            if Instant::now() > deadline {
                return Err(Error::new(409, "Annotation lookup timed out"));
            }
            offset = next
                .as_u64()
                .filter(|next| *next > offset && *next < 8192)
                .ok_or_else(|| Error::new(409, "Invalid annotation page"))?;
            page = self.request(
                runtime,
                json!({"type":"inspect_tree_page","generation":generation,"offset":offset}),
                false,
            )?;
        }
        Err(Error::new(409, "Annotation tree exceeded its page bound"))
    }

    fn context(&self, runtime: &str, generation: &Value, selected: &Value) -> Value {
        let mut text = selected["text"].as_str().unwrap_or_default().to_owned();
        let mut frames = vec![frame(selected)];
        let mut truncated = false;
        let mut index = HashMap::new();
        let mut descendants = HashSet::new();
        let mut covered = HashSet::new();
        let mut fragments = Vec::new();
        let mut text_length = 0;
        let mut found = false;
        let mut offset = 0;
        let deadline = Instant::now() + Duration::from_millis(1500);
        let scan = (|| -> Result<()> {
            for _ in 0..32 {
                let page = self.request(
                    runtime,
                    json!({"type":"inspect_tree_page","generation":generation,"offset":offset}),
                    false,
                )?;
                let snapshot = &page["snapshot"];
                if snapshot["capture"] != *generation {
                    return Err(Error::new(409, "Copy capture changed"));
                }
                let elements = snapshot["elements"]
                    .as_array()
                    .ok_or_else(|| Error::new(409, "Invalid copy page"))?;
                let mut done = false;
                for element in elements {
                    let id = element["id"]
                        .as_str()
                        .ok_or_else(|| Error::new(409, "Invalid copy element"))?;
                    let mut ancestry = frame(element);
                    ancestry["parent"] = element["parent"].clone();
                    index.insert(id.to_owned(), ancestry);
                    let parent = element["parent"].as_str().unwrap_or_default();
                    if element["id"] == selected["id"] {
                        found = true;
                    } else if found && !descendants.contains(parent) {
                        done = true;
                        break;
                    }
                    if !found {
                        continue;
                    }
                    descendants.insert(id.to_owned());
                    if element["accessibility"]["hidden"] == true || covered.contains(parent) {
                        covered.insert(id.to_owned());
                    } else if !element["text"].is_null() {
                        let fragment = element["text"]
                            .as_str()
                            .unwrap_or_default()
                            .chars()
                            .take(256)
                            .collect::<String>();
                        if !fragment.is_empty() {
                            text_length += fragment.chars().count();
                            fragments.push(fragment);
                        }
                        covered.insert(id.to_owned());
                    }
                    if text_length >= 256 {
                        done = true;
                        break;
                    }
                }
                let next = &page["next"];
                if done || next.is_null() {
                    return Ok(());
                }
                if Instant::now() > deadline {
                    return Err(Error::new(409, "Copy lookup timed out"));
                }
                offset = next
                    .as_u64()
                    .filter(|next| *next > offset && *next < 8192)
                    .ok_or_else(|| Error::new(409, "Invalid copy page"))?;
            }
            Err(Error::new(409, "Copy tree exceeded its page bound"))
        })();
        truncated |= scan.is_err();
        if !fragments.is_empty() {
            text = fragments.join(" ").chars().take(256).collect();
        }
        if found {
            let mut parent = selected["parent"].as_str().unwrap_or_default().to_owned();
            let mut visited =
                HashSet::from([selected["id"].as_str().unwrap_or_default().to_owned()]);
            while !parent.is_empty() && frames.len() < 20 {
                if !visited.insert(parent.clone()) {
                    truncated = true;
                    break;
                }
                let Some(ancestor) = index.get(&parent) else {
                    truncated = true;
                    break;
                };
                frames.push(fields(ancestor, &["name", "source"]));
                parent = ancestor["parent"].as_str().unwrap_or_default().to_owned();
            }
            if !parent.is_empty() && frames.len() == 20 {
                truncated = true;
            }
        } else {
            truncated = true;
        }
        json!({"text":text,"frames":frames,"truncated":truncated})
    }
}
fn frame(element: &Value) -> Value {
    let name = element["view"]["tag"]
        .as_str()
        .filter(|tag| !tag.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            element["kind"]
                .as_str()
                .unwrap_or("Element")
                .rsplit("::")
                .next()
                .unwrap_or("Element")
                .replace(['<', '>'], "")
        });
    json!({"name":name,"source":element["source"]})
}
