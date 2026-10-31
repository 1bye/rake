//! Captured layout checks and paint-order hints, without claiming GPU hitbox coverage.

use super::Session;
use crate::cdp::{TIMEOUT, semantic, wire::Error};
use rake_protocol::{
    control::{Query, Reply},
    inspection::{
        Element, Layout, Snapshot,
        preview::{Point, Window},
    },
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

impl Session {
    pub(super) fn actionability(&mut self, params: &Value) -> Result<Value, Error> {
        semantic::selector(params)?;
        let target = self.native_input.target(&self.instance)?;
        self.native_input.window = Some(target.window);
        let presentation = self.refresh_window(target.window)?;
        let projected = semantic::project(
            &presentation,
            &self.dom,
            &json!({"query":params.get("query").cloned().unwrap_or_else(||json!({})),"limit":2}),
        )?;
        if projected["totalMatches"] != 1 {
            return Err(Error::new(
                -32000,
                format!(
                    "Actionability requires one element; found {}",
                    projected["totalMatches"]
                ),
            ));
        }
        let element = &projected["elements"][0];
        let captured = presentation
            .tree
            .elements
            .iter()
            .find(|e| {
                self.dom
                    .node_id(&e.id)
                    .is_some_and(|id| element["nodeId"] == id)
            })
            .ok_or_else(|| Error::new(-32010, "Element identity is no longer retained"))?;
        let mut result = diagnose(&presentation.tree, captured, &target);
        result["element"] = element.clone();
        result["runtimeId"] = json!(self.instance.info.runtime_id);
        result["windowId"] = json!(target.window);
        result["frameSequence"] = json!(presentation.frame);
        result["revision"] = json!({"capture":presentation.revision.capture,"metadata":presentation.revision.metadata});
        if result["canDispatch"] == true {
            let point = Point {
                window: target.window,
                native_id: target.native_id,
                generation: presentation.revision.capture.clone(),
                geometry: None,
                x: result["point"]["x"]
                    .as_i64()
                    .expect("integer diagnostic point") as i32,
                y: result["point"]["y"]
                    .as_i64()
                    .expect("integer diagnostic point") as i32,
            };
            match crate::terminal::query(&self.instance, Query::PreviewElement { point }, TIMEOUT) {
                Ok(Reply::Element(hit)) => {
                    let relation = relationship(&presentation.tree, captured, &hit);
                    result["occlusion"]["capturedHit"] = json!({"nodeId":self.dom.node_id(&hit.id),"id":hit.semantic_id,"type":hit.display_name(),"relation":relation});
                    if relation == "other" {
                        result["notes"].as_array_mut().expect("diagnostic notes").push(reason("possibly_covered","Another captured boundary is painted at the dispatch point; native pointer interception is unknown"));
                    }
                }
                Err(error)
                    if matches!(
                        error.downcast_ref::<rake_client::Error>(),
                        Some(rake_client::Error::Control(
                            rake_protocol::control::ControlError::Inspection(
                                rake_protocol::inspection::InspectionError::StaleGeneration
                            )
                        ))
                    ) =>
                {
                    return Err(Error::new(
                        -32010,
                        "Capture changed while checking actionability; resolve the element again",
                    ));
                }
                Err(error) => {
                    result["occlusion"]["error"] =
                        json!(error.to_string().chars().take(1024).collect::<String>())
                }
                Ok(_) => result["occlusion"]["error"] = json!("Unexpected hit-test reply"),
            }
        }
        Ok(result)
    }
}

fn reason(code: &str, message: &str) -> Value {
    json!({"code":code,"message":message})
}
fn intersection(a: Layout, b: Layout) -> Layout {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    Layout {
        x,
        y,
        width: (a.x + a.width).min(b.x + b.width) - x,
        height: (a.y + a.height).min(b.y + b.height) - y,
    }
}
fn finite(bounds: Layout) -> bool {
    [bounds.x, bounds.y, bounds.width, bounds.height]
        .into_iter()
        .all(f32::is_finite)
}

fn diagnose(tree: &Snapshot, element: &Element, window: &Window) -> Value {
    let parents: BTreeMap<_, _> = tree.elements.iter().map(|e| (e.id.as_str(), e)).collect();
    let mut blockers = Vec::new();
    let mut notes = Vec::new();
    let mut current = Some(element);
    let mut depth = 0;
    while let Some(node) = current {
        if let Some(ax) = &node.accessibility {
            if ax.hidden {
                blockers.push(reason(
                    if depth == 0 {
                        "hidden"
                    } else {
                        "ancestor_hidden"
                    },
                    "Captured accessibility marks the element or its ancestor hidden",
                ));
            }
            if ax.disabled {
                blockers.push(reason(
                    if depth == 0 {
                        "disabled"
                    } else {
                        "ancestor_disabled"
                    },
                    "Captured accessibility marks the element or its ancestor disabled",
                ));
            }
        }
        depth += 1;
        if depth > tree.elements.len() {
            notes.push(reason(
                "ancestry_unknown",
                "Captured parent chain is cyclic",
            ));
            break;
        }
        current = node
            .parent
            .as_deref()
            .and_then(|id| parents.get(id).copied());
        if node.parent.is_some() && current.is_none() {
            notes.push(reason("ancestry_unknown", "Captured parent is unavailable"));
        }
    }
    let mut point = Value::Null;
    match element.layout.filter(|b| finite(*b)) {
        None => blockers.push(reason(
            "layout_unavailable",
            "No finite captured layout is available",
        )),
        Some(bounds) if bounds.width <= 0. || bounds.height <= 0. => blockers.push(reason(
            "zero_size",
            "The captured element has no positive area",
        )),
        Some(bounds) => {
            let viewport = Layout {
                x: 0.,
                y: 0.,
                width: window.content.width,
                height: window.content.height,
            };
            let mut visible = intersection(bounds, viewport);
            if let Some(mask) = element.content_mask.filter(|b| finite(*b)) {
                visible = intersection(visible, mask);
            } else {
                notes.push(reason(
                    "clipping_unknown",
                    "No finite content mask was captured",
                ));
            }
            if visible.width <= 0. || visible.height <= 0. {
                blockers.push(reason(
                    "fully_clipped",
                    "The element is outside the captured visible viewport; scroll it into view",
                ));
            } else {
                if visible != bounds {
                    notes.push(reason(
                        "partially_clipped",
                        "The dispatch point uses the visible intersection",
                    ));
                }
                let min_x = visible.x.ceil();
                let min_y = visible.y.ceil();
                let max_x = (visible.x + visible.width).ceil() - 1.;
                let max_y = (visible.y + visible.height).ceil() - 1.;
                if min_x > max_x || min_y > max_y {
                    blockers.push(reason(
                        "subpixel_target",
                        "The visible area contains no whole logical-point coordinate",
                    ));
                } else {
                    point = json!({"x":(visible.x+visible.width/2.).round().clamp(min_x,max_x) as i32,"y":(visible.y+visible.height/2.).round().clamp(min_y,max_y) as i32});
                }
            }
        }
    }
    if element.accessibility.is_none() {
        notes.push(reason(
            "enabled_unknown",
            "No captured accessibility enabled state is available",
        ));
    }
    if window.occluded {
        notes.push(reason(
            "native_window_occluded",
            "WindowServer reports this window covered; synthetic native dispatch remains available",
        ));
    }
    json!({"canDispatch":blockers.is_empty(),"blockers":blockers,"notes":notes,"point":point,
        "occlusion":{"status":"unknown","scope":"captured paint order; opaque hitboxes, pointer pass-through and final GPU occlusion are unavailable"}})
}

fn relationship(tree: &Snapshot, target: &Element, hit: &Element) -> &'static str {
    if target.id == hit.id {
        return "self";
    }
    let parents: BTreeMap<_, _> = tree
        .elements
        .iter()
        .map(|e| (e.id.as_str(), e.parent.as_deref()))
        .collect();
    let mut parent = hit.parent.as_deref();
    for _ in 0..tree.elements.len() {
        let Some(id) = parent else {
            break;
        };
        if id == target.id {
            return "descendant";
        }
        parent = parents.get(id).copied().flatten();
    }
    "other"
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostics_distinguish_hidden_disabled_clipped_and_fractional_targets() {
        let mut tree:Snapshot=serde_json::from_value(json!({"status":"captured","capture":"1","router_revision":0,"omitted":0,"coverage":"test","elements":[{"id":"a","kind":"Button","metadata_truncated":false,"layout":{"x":0,"y":0,"width":100,"height":30},"accessibility":{"hidden":false,"disabled":true,"focusable":false,"actions":[]}}]})).unwrap();
        let window:Window=serde_json::from_value(json!({"window":1,"native_id":2,"generation":"1","frame":{"x":0,"y":0,"width":100,"height":100},"content":{"x":0,"y":0,"width":100,"height":100},"translucent":false})).unwrap();
        assert_eq!(
            diagnose(&tree, &tree.elements[0], &window)["blockers"][0]["code"],
            "disabled"
        );
        tree.elements[0].accessibility.as_mut().unwrap().disabled = false;
        tree.elements[0].layout.as_mut().unwrap().y = 200.;
        assert_eq!(
            diagnose(&tree, &tree.elements[0], &window)["blockers"][0]["code"],
            "fully_clipped"
        );
        tree.elements[0].layout = Some(Layout {
            x: 99.2,
            y: 0.,
            width: 1.,
            height: 10.,
        });
        let report = diagnose(&tree, &tree.elements[0], &window);
        assert_eq!(report["canDispatch"], false);
        assert_eq!(report["blockers"][0]["code"], "subpixel_target");
        tree.elements[0].layout = Some(Layout {
            x: 99.,
            y: 0.,
            width: 2.,
            height: 10.,
        });
        assert_eq!(
            diagnose(&tree, &tree.elements[0], &window)["point"]["x"],
            99
        );
        tree.elements[0].accessibility.as_mut().unwrap().hidden = true;
        assert_eq!(
            diagnose(&tree, &tree.elements[0], &window)["blockers"][0]["code"],
            "hidden"
        );
    }
}
