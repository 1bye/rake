//! Pure request-lifecycle translation. Headers, bodies and JS stacks are unavailable.

use super::wire::Event;
use rake::diagnostics::Record;
use serde_json::{Value, json};

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

    let (method, params) = match phase {
        "started" => (
            "Network.requestWillBeSent",
            json!({
            "requestId":request_id, "loaderId":"rake", "documentURL":"rake://runtime",
            "request":{"url":url,"method":f.get("method").and_then(Value::as_str).unwrap_or_default(),
                "headers":{},"initialPriority":"Medium","referrerPolicy":"no-referrer"},
            "timestamp":timestamp,"initiator":{"type":"other"},"type":"Fetch","frameId":"rake",
            "hasUserGesture":false}),
        ),
        "headers_received" => (
            "Network.responseReceived",
            json!({
            "requestId":request_id,"loaderId":"rake","timestamp":timestamp,"type":"Fetch",
            "response":{"url":url,"status":f.get("status"),"statusText":"","headers":{},"mimeType":"",
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

    vec![Event::new(method, params)]
}

#[cfg(test)]
mod tests {
    use super::*;

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
