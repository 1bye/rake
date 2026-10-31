//! Request-lifecycle translation; absent engine measurements remain unavailable.

use super::wire::{Error, Event};
use base64::Engine;
use rake_diagnostics::Record;
use serde_json::{Value, json};

pub(super) fn body(value: Value, response: bool) -> Result<Value, Error> {
    let body: rake_diagnostics::http::Body = serde_json::from_value(value).map_err(|_| {
        Error::new(
            -32000,
            "Body capture unavailable or evicted (64 requests / 1 MiB)",
        )
    })?;

    if body.truncated {
        return Err(Error::new(-32000, "Body exceeds the 64 KiB capture limit"));
    }

    if !body.complete {
        return Err(Error::new(
            -32000,
            format!(
                "Body incomplete: {}",
                body.failure.as_deref().unwrap_or("still being consumed")
            ),
        ));
    }

    if response {
        let (body, encoded) = match String::from_utf8(body.bytes) {
            Ok(text) => (text, false),
            Err(error) => (
                base64::engine::general_purpose::STANDARD.encode(error.as_bytes()),
                true,
            ),
        };

        Ok(json!({"body":body,"base64Encoded":encoded}))
    } else {
        let text = String::from_utf8(body.bytes).map_err(|_| {
            Error::new(-32000, "Request body is binary; text post data unavailable")
        })?;

        Ok(json!({"postData":text}))
    }
}

pub(super) fn translate(record: &Record) -> Vec<Event> {
    if record.kind != "request" {
        return Vec::new();
    }

    let f = &record.fields;

    let Some(id) = f.get("request_id") else {
        return Vec::new();
    };

    let request_id = id
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| id.to_string());

    let timestamp = record.elapsed_us as f64 / 1e6;
    let phase = f.get("phase").and_then(Value::as_str).unwrap_or_default();
    let url = f.get("url").and_then(Value::as_str).unwrap_or_default();

    let (method, mut params) = match phase {
        "started" => (
            "Network.requestWillBeSent",
            json!({
            "requestId":request_id, "loaderId":"rake", "documentURL":"rake://runtime",
            "request":{"url":url,"method":f.get("method").and_then(Value::as_str).unwrap_or_default(),
                "hasPostData":f.get("has_body").and_then(Value::as_bool).unwrap_or(false),
                "headers":f.get("request_headers").cloned().unwrap_or(json!({})),"initialPriority":"Medium","referrerPolicy":"no-referrer"},
            "timestamp":f.get("request_start_us").and_then(Value::as_u64).map(|us| us as f64 / 1e6).unwrap_or(timestamp),"initiator":{"type":"other"},"type":"Fetch","frameId":"rake",
            "hasUserGesture":false}),
        ),
        "headers_received" => (
            "Network.responseReceived",
            json!({
            "requestId":request_id,"loaderId":"rake","timestamp":timestamp,"type":"Fetch",
            "response":{"url":f.get("response_url").and_then(Value::as_str).unwrap_or(url),"status":f.get("status"),
                "statusText":f.get("status_text").and_then(Value::as_str).unwrap_or_default(),
                "headers":f.get("response_headers").cloned().unwrap_or(json!({})),
                "mimeType":f.get("response_headers").and_then(|h| h["content-type"].as_str()).unwrap_or_default().split(';').next().unwrap_or_default(),
                "protocol":f.get("protocol").and_then(Value::as_str).unwrap_or_default(),
                "connectionReused":false,"connectionId":0,"encodedDataLength":0,"securityState":"unknown"},
            "hasExtraInfo":false,"frameId":"rake"}),
        ),
        "body_progress" => (
            "Network.dataReceived",
            json!({"requestId":request_id,
            "timestamp":timestamp,"dataLength":f.get("bytes"),"encodedDataLength":0}),
        ),
        "completed" => (
            "Network.loadingFinished",
            json!({"requestId":request_id,
            "timestamp":timestamp,"encodedDataLength":0}),
        ),
        "failed" | "timed_out" | "cancelled" | "abandoned" => (
            "Network.loadingFailed",
            json!({
            "requestId":request_id,"timestamp":timestamp,"type":"Fetch","errorText":phase,
            "canceled": matches!(phase,"cancelled" | "abandoned")}),
        ),
        _ => return Vec::new(),
    };

    if phase == "headers_received"
        && let Some(duration) = f.get("duration_us").and_then(Value::as_u64)
    {
        let start_us = f
            .get("request_start_us")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| record.elapsed_us.saturating_sub(duration));

        let start = start_us as f64 / 1e6;
        let headers_ms = record.elapsed_us.saturating_sub(start_us) as f64 / 1000.0;

        params["response"]["timing"] = json!({
            "requestTime":start,"proxyStart":-1,"proxyEnd":-1,
            "dnsStart":-1,"dnsEnd":-1,"connectStart":-1,"connectEnd":-1,
            "sslStart":-1,"sslEnd":-1,"workerStart":-1,"workerReady":-1,
            "workerFetchStart":-1,"workerRespondWithSettled":-1,
            "sendStart":-1,"sendEnd":-1,"pushStart":0,"pushEnd":0,
            "receiveHeadersStart":headers_ms,"receiveHeadersEnd":headers_ms
        });
    }

    if phase == "started"
        && let Some(source) = &record.source
    {
        let mut context = record.spans.join(" → ");

        for key in ["command_id", "operation_id", "task_id", "generation_id"] {
            if let Some(value) = f.get(key) {
                context.push_str(&format!(" {key}={}", value.as_str().unwrap_or_default()));
            }
        }

        if let Some(id) = record.span_id {
            context.push_str(&format!(" span={id}"));
        }

        params["initiator"] = json!({"type":"script","stack":{"description":"Rust request origin (not a JavaScript stack)",
            "callFrames":[{"functionName":context.trim(),"scriptId":"","url":super::console::source_url(source),
            "lineNumber":source.line.saturating_sub(1),"columnNumber":0}]}});
    }

    vec![Event::new(method, params)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodies_preserve_bytes_and_reject_incomplete_captures() {
        assert_eq!(
            body(
                json!({"bytes":[104,105],"complete":true,"truncated":false,"failure":null}),
                true
            )
            .unwrap(),
            json!({"body":"hi","base64Encoded":false})
        );
        assert_eq!(
            body(
                json!({"bytes":[0,255],"complete":true,"truncated":false,"failure":null}),
                true
            )
            .unwrap(),
            json!({"body":"AP8=","base64Encoded":true})
        );

        for (complete, truncated, failure) in [
            (false, false, None),
            (false, false, Some("cancelled")),
            (true, true, None),
        ] {
            assert!(
                body(
                    json!({"bytes":[],"complete":complete,"truncated":truncated,"failure":failure}),
                    true
                )
                .is_err()
            );
        }

        assert!(body(Value::Null, false).is_err());
    }

    #[test]
    fn timings_and_rust_origin_preserve_real_context() {
        let mut record: Record = serde_json::from_value(json!({"sequence":1,"elapsed_us":1250000,
            "source":{"file":"src/fetch.rs","line":17},"span_id":8,
            "level":"INFO","target":"rake::http","kind":"request","spans":["command.execute","http.request"],"fields":{
            "request_id":7,"phase":"started","command_id":"demo.fetch","operation_id":"op-3","task_id":"task-2","url":"https://example.test/"}})).unwrap();

        let event = translate(&record).remove(0);
        let origin = &event.params["initiator"]["stack"]["callFrames"][0];

        assert_eq!(origin["lineNumber"], 16);
        assert!(
            origin["functionName"]
                .as_str()
                .unwrap()
                .contains("command_id=demo.fetch operation_id=op-3 task_id=task-2")
        );
        record
            .fields
            .insert("phase".into(), json!("headers_received"));
        record.fields.insert("duration_us".into(), json!(250000));
        let event = translate(&record).remove(0);

        assert_eq!(event.params["response"]["timing"]["requestTime"], 1.0);
        assert_eq!(
            event.params["response"]["timing"]["receiveHeadersEnd"],
            250.0
        );
        assert_eq!(event.params["response"]["timing"]["dnsStart"], -1);
    }

    #[test]
    fn preserved_bodies_are_bounded_and_old_ids_cannot_retarget() {
        let mut bodies = Bodies::default();
        let data = json!({"bytes":[104,105],"complete":true,"truncated":false,"failure":null});

        bodies.insert("old:1".into(), [Value::Null, data.clone()]);
        assert_eq!(
            body(bodies.get("old:1", true).unwrap(), true).unwrap()["body"],
            "hi"
        );
        assert!(local_id("old:1", "new").is_err());
        assert_eq!(local_id("new:1", "new").unwrap(), 1);

        for id in 1..70 {
            bodies.insert(format!("new:{id}"), [Value::Null, data.clone()]);
        }

        assert!(bodies.get("old:1", true).is_none());
        assert!(bodies.entries.len() <= 64);
        let large = serde_json::to_value(rake_diagnostics::http::Body {
            bytes: vec![0; 65536],
            complete: true,
            ..Default::default()
        })
        .unwrap();

        for id in 70..100 {
            bodies.insert(format!("new:{id}"), [large.clone(), large.clone()]);
        }

        assert!(bodies.bytes <= 1024 * 1024);
    }

    #[test]
    fn response_search_handles_case_regex_and_binary_failures() {
        let body = json!({"body":"First\nhello World\nhello again","base64Encoded":false});

        assert_eq!(
            search(&body, &json!({"query":"world"})).unwrap()["result"][0]["lineNumber"],
            1
        );
        assert_eq!(
            search(&body, &json!({"query":"world","caseSensitive":true})).unwrap()["result"],
            json!([])
        );
        assert_eq!(
            search(&body, &json!({"query":"^hello","isRegex":true})).unwrap()["result"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert!(search(&body, &json!({"query":"[","isRegex":true})).is_err());
        assert!(
            search(
                &json!({"body":"AP8=","base64Encoded":true}),
                &json!({"query":"x"})
            )
            .is_err()
        );
    }

    #[test]
    fn every_phase_preserves_identity_time_and_cancellation() {
        for (phase, method) in [
            ("started", "requestWillBeSent"),
            ("headers_received", "responseReceived"),
            ("body_progress", "dataReceived"),
            ("completed", "loadingFinished"),
            ("failed", "loadingFailed"),
            ("timed_out", "loadingFailed"),
            ("cancelled", "loadingFailed"),
            ("abandoned", "loadingFailed"),
        ] {
            let record: Record = serde_json::from_value(json!({"sequence":1,"elapsed_us":1250000,
                "level":"INFO","target":"rake::http","kind":"request","spans":[],"fields":{
                    "request_id":7,"phase":phase,"url":"https://example.test/redacted","method":"GET",
                    "status":200,"bytes":3,"received_bytes":9}})).unwrap();

            let events = translate(&record);

            assert_eq!(events.len(), 1);
            assert_eq!(events[0].method, format!("Network.{method}"));
            assert_eq!(events[0].params["requestId"], "7");
            assert_eq!(events[0].params["timestamp"], 1.25);

            if method == "loadingFailed" {
                assert_eq!(events[0].params["errorText"], phase);
                assert_eq!(
                    events[0].params["canceled"],
                    matches!(phase, "cancelled" | "abandoned")
                );
            }
        }
    }
}

/// Completed bodies survive runtime replacement for Chrome's Preserve log, within one connection.
#[derive(Default)]
pub(super) struct Bodies {
    entries: std::collections::VecDeque<(String, [Option<rake_diagnostics::http::Body>; 2], usize)>,
    bytes: usize,
}

impl Bodies {
    pub fn insert(&mut self, id: String, bodies: [Value; 2]) {
        if self.entries.iter().any(|entry| entry.0 == id) {
            return;
        }

        let bodies =
            bodies.map(|body| serde_json::from_value::<rake_diagnostics::http::Body>(body).ok());

        let bytes = bodies
            .iter()
            .flatten()
            .map(|body| body.bytes.len())
            .sum::<usize>();

        if bytes > 1024 * 1024 {
            return;
        }

        while self.entries.len() >= 64 || self.bytes.saturating_add(bytes) > 1024 * 1024 {
            if let Some((_, _, bytes)) = self.entries.pop_front() {
                self.bytes -= bytes;
            } else {
                break;
            }
        }

        self.bytes += bytes;
        self.entries.push_back((id, bodies, bytes));
    }

    pub fn get(&self, id: &str, response: bool) -> Option<Value> {
        self.entries
            .iter()
            .find(|entry| entry.0 == id)
            .map(|entry| serde_json::to_value(&entry.1[usize::from(response)]).unwrap_or_default())
    }
}

pub(super) fn local_id(id: &str, runtime: &str) -> Result<u64, Error> {
    let (owner, id) = id
        .split_once(':')
        .ok_or_else(|| Error::new(-32602, "Invalid request ID"))?;

    if owner != runtime {
        return Err(Error::new(
            -32000,
            "Request belongs to an expired runtime; body is no longer retained",
        ));
    }

    id.parse()
        .map_err(|_| Error::new(-32602, "Invalid request ID"))
}

pub(super) fn search(body: &Value, params: &Value) -> Result<Value, Error> {
    let text = body["body"]
        .as_str()
        .filter(|_| body["base64Encoded"] == false)
        .ok_or_else(|| Error::new(-32000, "Search requires a complete UTF-8 response"))?;

    let query = params["query"]
        .as_str()
        .ok_or_else(|| Error::new(-32602, "Search query required"))?;

    let pattern = if params["isRegex"].as_bool().unwrap_or(false) {
        query.to_owned()
    } else {
        regex::escape(query)
    };

    let matcher = regex::RegexBuilder::new(&pattern)
        .case_insensitive(!params["caseSensitive"].as_bool().unwrap_or(false))
        .size_limit(1024 * 1024)
        .build()
        .map_err(|_| Error::new(-32602, "Invalid or oversized search expression"))?;

    let result: Vec<Value> = text
        .lines()
        .enumerate()
        .filter(|(_, line)| matcher.is_match(line))
        .take(1000)
        .map(|(number, line)| json!({"lineNumber":number,"lineContent":line}))
        .collect();

    Ok(json!({"result":result}))
}
