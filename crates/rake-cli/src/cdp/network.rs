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
            "timestamp":timestamp,"initiator":{"type":"other"},"type":"Fetch","frameId":"rake",
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
            "timestamp":timestamp,"encodedDataLength":f.get("received_bytes")}),
        ),
        "failed" | "timed_out" | "cancelled" | "abandoned" => (
            "Network.loadingFailed",
            json!({
            "requestId":request_id,"timestamp":timestamp,"type":"Fetch","errorText":phase,
            "canceled": matches!(phase,"cancelled" | "abandoned")}),
        ),
        _ => return Vec::new(),
    };

    if phase == "headers_received" {
        if let Some(duration) = f.get("duration_us").and_then(Value::as_u64) {
            let start = record.elapsed_us.saturating_sub(duration) as f64 / 1e6;
            let headers_ms = duration as f64 / 1000.0;
            params["response"]["timing"] = json!({
                "requestTime":start,"proxyStart":-1,"proxyEnd":-1,
                "dnsStart":-1,"dnsEnd":-1,"connectStart":-1,"connectEnd":-1,
                "sslStart":-1,"sslEnd":-1,"workerStart":-1,"workerReady":-1,
                "workerFetchStart":-1,"workerRespondWithSettled":-1,
                "sendStart":-1,"sendEnd":-1,"pushStart":0,"pushEnd":0,
                "receiveHeadersStart":headers_ms,"receiveHeadersEnd":headers_ms
            });
        }
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
