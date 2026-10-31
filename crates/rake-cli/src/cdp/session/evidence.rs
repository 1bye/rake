//! One bounded diagnostic bundle; independent samples never imply an atomic GPU frame.

use super::Session;
use crate::cdp::{
    TIMEOUT, semantic,
    wire::{self, Error, Request},
};
use rake_protocol::{
    control::{Query, Reply},
    inspection::SourceLocation,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Options {
    query: Option<Value>,
    limit: Option<usize>,
    root_node_id: Option<i32>,
    screenshot: Option<bool>,
    diagnostics_after: Option<u64>,
    #[serde(rename = "rakeRuntimeId")]
    _runtime: Option<String>,
}

impl Session {
    pub(super) fn evidence(&mut self, params: &Value) -> Result<Value, Error> {
        semantic::root_node(params)?;
        let options: Options = serde_json::from_value(params.clone())
            .map_err(|e| Error::new(-32602, e.to_string()))?;
        let limit = options.limit.unwrap_or(32);
        let mut tree_params = json!({"limit":limit});
        if let Some(root) = options.root_node_id {
            tree_params["rootNodeId"] = json!(root);
        }
        semantic::root_node(&tree_params)?;
        if !(1..=64).contains(&limit) {
            return Err(Error::new(-32602, "Evidence limit must be 1..64"));
        }
        if let Some(query) = &options.query {
            semantic::selector(&json!({"query":query}))?;
        }

        let started = Instant::now();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |t| t.as_millis());
        let window = self.native_input.target(&self.instance)?.window;
        self.native_input.window = Some(window);
        let presentation = self.refresh_window(window)?;
        let mut tree = semantic::project(&presentation, &self.dom, &tree_params)?;
        tree["runtimeId"] = json!(self.instance.info.runtime_id);
        let query = options.query.or_else(|| {
            self.dom
                .inspected_node()
                .ok()
                .map(|node| json!({"nodeId":node}))
        });
        let selected = (|| {
            let query = query.ok_or_else(|| {
                Error::new(-32000, "Supply query or select an element in DevTools")
            })?;
            let mut selected_params = tree_params.clone();
            selected_params["limit"] = json!(2);
            selected_params["query"] = query;
            let snapshot = semantic::project(&presentation, &self.dom, &selected_params)?;
            if snapshot["totalMatches"] != 1 {
                return Err(Error::selection(
                    snapshot["totalMatches"].as_u64().unwrap_or(0),
                    "Evidence selection",
                ));
            }
            let mut element = snapshot["elements"][0].clone();
            if !element["source"].is_null() {
                let source: SourceLocation = serde_json::from_value(element["source"].clone())
                    .map_err(|e| Error::new(-32000, e.to_string()))?;
                element["source"] = self.source_descriptor(&source);
            }
            Ok(element)
        })();
        let source = selected.as_ref().map_err(Clone::clone).and_then(|element| {
            let source: SourceLocation = serde_json::from_value(element["source"].clone())
                .map_err(|_| Error::new(-32000, "Selected element has no Rust source"))?;
            snippet(&source)
        });
        let diagnostics = match crate::terminal::query(
            &self.instance,
            Query::Diagnostics {
                after: options.diagnostics_after.unwrap_or(0),
                kind: None,
            },
            TIMEOUT,
        ) {
            Ok(Reply::Diagnostics(records)) => Ok(recent(&records)),
            Ok(_) => Err(Error::new(-32000, "Unexpected diagnostics reply")),
            Err(error) => Err(Error::new(-32000, error.to_string())),
        };
        let screenshot = if options.screenshot.unwrap_or(true) {
            section(
                self.handle(&Request {
                    id: 0,
                    method: "Page.captureScreenshot".into(),
                    params: json!({"format":"jpeg","quality":70}),
                }),
                512 * 1024,
            )
        } else {
            json!({"status":"omitted","reason":"disabled by caller"})
        };
        let mut bundle = json!({"runtimeId":self.instance.info.runtime_id,"windowId":window,
            "startedAtUnixMs":timestamp,"durationMs":started.elapsed().as_millis(),
            "frameSequence":presentation.frame,"revision":{"capture":presentation.revision.capture,"metadata":presentation.revision.metadata},
            "sampling":"tree and selection share one inspection draw; diagnostics, source file and image are independent samples",
            "sections":{"tree":section(Ok(tree),192*1024),"selected":section(selected,16*1024),"source":section(source,12*1024),"diagnostics":section(diagnostics,64*1024),"screenshot":screenshot}});
        bundle["complete"] = json!(bundle["sections"].as_object().is_some_and(|sections| {
            sections
                .values()
                .all(|s| s["status"] == "ok" || s["status"] == "omitted")
        }));
        Ok(bundle)
    }
}

fn section(result: Result<Value, Error>, budget: usize) -> Value {
    match result {
        Ok(value) if wire::serialized_len(&value).is_ok_and(|size| size <= budget) => {
            json!({"status":if partial(&value){"partial"}else{"ok"},"data":value})
        }
        Ok(_) => {
            json!({"status":"truncated","error":{"code":-32000,"message":"Section exceeds evidence byte budget"},"budgetBytes":budget})
        }
        Err(mut error) => {
            let mut end = error.message.len().min(4096);
            while !error.message.is_char_boundary(end) {
                end -= 1;
            }
            error.message.truncate(end);
            json!({"status":"error","error":error})
        }
    }
}

fn partial(value: &Value) -> bool {
    value["queryComplete"] == false
        || value["hasMore"] == true
        || value["truncated"] == true
        || [
            "omitted",
            "omittedRetained",
            "dropped",
            "evicted",
            "outputDropped",
        ]
        .iter()
        .any(|key| value[key].as_u64().is_some_and(|n| n > 0))
}

fn snippet(source: &SourceLocation) -> Result<Value, Error> {
    let text = crate::source::read(source).map_err(|e| Error::new(-32000, e.to_string()))?;
    let start = source.line.saturating_sub(4) as usize;
    let excerpt = text
        .lines()
        .skip(start)
        .take(7)
        .collect::<Vec<_>>()
        .join("\n");
    let mut end = excerpt.len().min(8192);
    while !excerpt.is_char_boundary(end) {
        end -= 1;
    }
    Ok(
        json!({"file":source.file,"line":source.line,"column":source.column,"startLine":start+1,"text":&excerpt[..end],"truncated":end<excerpt.len()}),
    )
}

fn recent(snapshot: &rake_diagnostics::Snapshot) -> Value {
    let mut logs = Vec::new();
    let mut failures = Vec::new();
    let mut omitted = 0;
    let mut bytes = 0;
    for record in snapshot.records.iter().rev() {
        let failed = record.kind == "request"
            && (record
                .fields
                .get("status")
                .and_then(Value::as_u64)
                .is_some_and(|n| n >= 400)
                || record
                    .fields
                    .get("phase")
                    .and_then(Value::as_str)
                    .is_some_and(|s| {
                        matches!(s, "failed" | "timed_out" | "cancelled" | "abandoned")
                    }));
        if record.kind != "event" && !failed {
            continue;
        }
        let target = if failed { &mut failures } else { &mut logs };
        let size = wire::serialized_len(record).unwrap_or(usize::MAX);
        if target.len() >= 16 || size > 8192 || size > 48 * 1024 - bytes {
            omitted += 1;
            continue;
        }
        bytes += size;
        target.push(record);
    }
    logs.reverse();
    failures.reverse();
    json!({"logs":logs,"networkErrors":failures,"omittedRetained":omitted,
        "observedThrough":snapshot.records.last().map(|r|r.sequence),"dropped":snapshot.dropped,"evicted":snapshot.evicted,"outputDropped":snapshot.output_dropped,
        "scope":"retained runtime diagnostics; does not advance Console or Network cursors"})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn evidence_budgets_and_loss_are_explicit_and_do_not_hide_failures() {
        assert_eq!(
            section(Ok(json!({"large":"x".repeat(32)})), 8)["status"],
            "truncated"
        );
        assert_eq!(
            section(Err(Error::new(-32601, "unavailable")), 8)["status"],
            "error"
        );
        let snapshot:rake_diagnostics::Snapshot=serde_json::from_value(json!({"dropped":2,"evicted":3,"output_dropped":4,"records":[
            {"sequence":1,"elapsed_us":1,"level":"WARN","target":"app","kind":"event","spans":[],"fields":{"message":"example"}},
            {"sequence":2,"elapsed_us":2,"level":"INFO","target":"http","kind":"request","spans":[],"fields":{"status":503}},
            {"sequence":3,"elapsed_us":3,"level":"INFO","target":"http","kind":"request","spans":[],"fields":{"phase":"failed"}},
            {"sequence":4,"elapsed_us":4,"level":"INFO","target":"app","kind":"event","spans":[],"fields":{"large":"🙂".repeat(3000)}}
        ]})).unwrap();
        let data = recent(&snapshot);
        assert_eq!(data["logs"].as_array().unwrap().len(), 1);
        assert_eq!(data["networkErrors"].as_array().unwrap().len(), 2);
        assert_eq!(
            section(Ok(json!({"hasMore":true})), 1024)["status"],
            "partial"
        );
        assert!(
            wire::serialized_len(&section(Err(Error::new(-32000, "🙂".repeat(100000))), 8192))
                .unwrap()
                < 8192
        );
        assert_eq!(data["omittedRetained"], 1);
        assert_eq!(data["dropped"], 2);
        assert_eq!(data["evicted"], 3);
    }
}
