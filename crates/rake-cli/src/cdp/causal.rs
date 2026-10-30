//! Bounded live spans and HTTP lifecycles on the calibrated GPUI recording clock.

use rake_diagnostics::{Record, Snapshot};
use rake_protocol::inspection::FrameClock;
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Causal {
    pub cursor: u64,
    offset: Option<i128>,
    uncertainty: u64,
    unrecorded: u64,
    pub loss: bool,
    spans: BTreeMap<u64, Start>,
    requests: BTreeMap<u64, Start>,
    bytes: usize,
}

struct Start {
    at: u64,
    label: String,
    lane: &'static str,
    properties: Vec<Value>,
    bytes: usize,
}

impl Causal {
    pub fn calibrate(&mut self, clock: &FrameClock) {
        self.offset = Some(i128::from(clock.diagnostics_us) - i128::from(clock.recording_us));
        self.cursor = clock.diagnostics_sequence;
        self.uncertainty = clock.uncertainty_us;
        self.unrecorded = clock.unrecorded;
    }

    pub fn enabled(&self) -> bool {
        self.offset.is_some()
    }

    pub fn metadata(&self) -> Value {
        json!({"available":self.enabled(),"calibrationWindowUs":self.uncertainty,
            "diagnosticsMinusRecordingUs":self.offset,"incompleteObservations":self.loss})
    }

    fn time(&self, us: u64) -> u64 {
        (i128::from(us) - self.offset.unwrap_or_default()).clamp(0, i128::from(u64::MAX)) as u64
    }

    pub fn observe(&mut self, snapshot: &Snapshot, sources: &BTreeMap<u64, String>) -> Vec<Value> {
        self.loss |= snapshot.dropped.saturating_sub(snapshot.evicted) > self.unrecorded;
        self.unrecorded = snapshot.dropped.saturating_sub(snapshot.evicted);
        let mut events = Vec::new();
        let cursor = self.cursor;
        let mut previous = cursor;
        for record in snapshot
            .records
            .iter()
            .filter(|record| record.sequence > cursor)
        {
            self.loss |= record.sequence > previous.saturating_add(1);
            previous = record.sequence;
            let properties = properties(record, sources.get(&record.sequence));
            match record.kind.as_str() {
                "span_open" => {
                    if let Some(id) = record.span_id {
                        let lane = lane(record);
                        let label = label(record);
                        self.insert(
                            id,
                            Start {
                                at: self.time(record.elapsed_us),
                                label,
                                lane,
                                properties,
                                bytes: 0,
                            },
                            false,
                        );
                    }
                }
                "span_close" => {
                    if let Some(id) = record.span_id {
                        let cached = self.remove(id, false);
                        if let Some(duration) = record.duration_us {
                            let start = cached.as_ref().map_or_else(
                                || self.time(record.elapsed_us.saturating_sub(duration)),
                                |start| start.at,
                            );
                            let end = self.time(record.elapsed_us);
                            let mut properties = properties;
                            properties.push(json!([
                                "Span start outside capture",
                                cached.is_none().to_string()
                            ]));
                            properties.push(json!([
                                "Measurement",
                                "asynchronous span lifetime; not CPU execution"
                            ]));
                            if let Some(parent) =
                                record.parent_span_id.and_then(|id| self.spans.get(&id))
                            {
                                let flow = format!(
                                    "span-{}-{id}",
                                    record.parent_span_id.unwrap_or_default()
                                );
                                events.push(json!({"name":"Captured span parent","cat":"rake.causal","ph":"s","id":flow,"pid":1,"tid":1,"ts":parent.at}));
                                events.push(json!({"name":"Captured span parent","cat":"rake.causal","ph":"f","bp":"e","id":flow,"pid":1,"tid":1,"ts":start}));
                            }
                            events.extend(super::frames::track(
                                &label(record),
                                start,
                                end.saturating_sub(start),
                                lane(record),
                                "secondary",
                                properties,
                            ));
                        }
                    }
                }
                "request" => {
                    let Some(id) = record.fields.get("request_id").and_then(Value::as_u64) else {
                        continue;
                    };
                    let phase = record
                        .fields
                        .get("phase")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let request_label =
                        format!("HTTP {} {}", text(record, "method"), text(record, "url"));
                    if phase == "started" {
                        let start = record
                            .fields
                            .get("request_start_us")
                            .and_then(Value::as_u64)
                            .unwrap_or(record.elapsed_us);
                        self.insert(
                            id,
                            Start {
                                at: self.time(start),
                                label: request_label,
                                lane: "HTTP lifecycles",
                                properties,
                                bytes: 0,
                            },
                            true,
                        );
                    } else if matches!(
                        phase,
                        "completed" | "failed" | "timed_out" | "cancelled" | "abandoned"
                    ) {
                        let cached = self.remove(id, true);
                        let duration = record
                            .fields
                            .get("duration_us")
                            .and_then(Value::as_u64)
                            .unwrap_or_default();
                        let start = cached.as_ref().map_or_else(
                            || {
                                self.time(
                                    record
                                        .fields
                                        .get("request_start_us")
                                        .and_then(Value::as_u64)
                                        .unwrap_or(record.elapsed_us.saturating_sub(duration)),
                                )
                            },
                            |start| start.at,
                        );
                        let end = self.time(record.elapsed_us);
                        let mut properties = properties;
                        properties.push(json!([
                            "Request start outside capture",
                            cached.is_none().to_string()
                        ]));
                        properties.push(json!([
                            "Measurement",
                            "request through application body consumption"
                        ]));
                        events.extend(super::frames::track(
                            &request_label,
                            start,
                            end.saturating_sub(start),
                            "HTTP lifecycles",
                            if phase == "completed" {
                                "primary"
                            } else {
                                "error"
                            },
                            properties,
                        ));
                    }
                }
                _ => {}
            }
        }
        if let Some(record) = snapshot.records.last() {
            self.cursor = self.cursor.max(record.sequence);
        }
        events
    }

    fn insert(&mut self, id: u64, mut start: Start, request: bool) {
        let _ = self.remove(id, request);
        start.bytes = start.label.len()
            + serde_json::to_vec(&start.properties).map_or(usize::MAX / 2, |data| data.len())
            + 128;
        if start.bytes > 128 * 1024 {
            self.loss = true;
            return;
        }
        while self.spans.len() + self.requests.len() >= 128 || self.bytes + start.bytes > 128 * 1024
        {
            let old = self.spans.pop_first().or_else(|| self.requests.pop_first());
            if let Some((_, start)) = old {
                self.bytes -= start.bytes;
                self.loss = true;
            } else {
                break;
            }
        }
        self.bytes += start.bytes;
        if request {
            self.requests.insert(id, start);
        } else {
            self.spans.insert(id, start);
        }
    }

    fn remove(&mut self, id: u64, request: bool) -> Option<Start> {
        let start = if request {
            self.requests.remove(&id)
        } else {
            self.spans.remove(&id)
        };
        if let Some(start) = &start {
            self.bytes -= start.bytes;
        }
        start
    }

    pub fn finish(&mut self, end: u64) -> Vec<Value> {
        let mut events = Vec::new();
        for (_, start) in std::mem::take(&mut self.spans)
            .into_iter()
            .chain(std::mem::take(&mut self.requests))
        {
            let mut properties = start.properties;
            properties.push(json!([
                "Incomplete",
                "end not observed before recording stopped"
            ]));
            events.extend(super::frames::track(
                &format!("Incomplete / {}", start.label),
                start.at,
                end.saturating_sub(start.at),
                start.lane,
                "secondary-light",
                properties,
            ));
        }
        self.bytes = 0;
        events
    }
}

fn text(record: &Record, key: &str) -> String {
    record
        .fields
        .get(key)
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string())
        })
        .unwrap_or_default()
        .chars()
        .take(256)
        .collect()
}

fn lane(record: &Record) -> &'static str {
    match record.spans.last().map(String::as_str) {
        Some("command.execute") => "Commands",
        Some("work.run") => "Tasks",
        _ => "Tracing spans",
    }
}

fn label(record: &Record) -> String {
    format!(
        "{} {} {}",
        record.spans.last().unwrap_or(&record.target),
        text(record, "command_id"),
        text(record, "task_id")
    )
    .trim()
    .chars()
    .take(512)
    .collect()
}

fn properties(record: &Record, source: Option<&String>) -> Vec<Value> {
    let mut result = vec![
        json!(["Diagnostic sequence", record.sequence.to_string()]),
        json!([
            "Span ID",
            record
                .span_id
                .map_or_else(|| "unavailable".into(), |id| id.to_string())
        ]),
        json!([
            "Parent span ID",
            record
                .parent_span_id
                .map_or_else(|| "unavailable".into(), |id| id.to_string())
        ]),
    ];
    for key in record.fields.keys().take(16) {
        result.push(json!([key, text(record, key)]));
    }
    if let Some(source) = source {
        result.push(json!(["Rust source URL", source]));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record(sequence: u64, elapsed: u64, kind: &str, id: Option<u64>, fields: Value) -> Record {
        serde_json::from_value(json!({"sequence":sequence,"elapsed_us":elapsed,"level":"INFO","target":"test","kind":kind,
            "spans":["command.execute"],"span_id":id,"duration_us":if kind=="span_close" {Some(2000)} else {None},"fields":fields})).unwrap()
    }
    fn clock() -> FrameClock {
        FrameClock {
            recording_us: 1000,
            diagnostics_us: 10000,
            uncertainty_us: 2,
            diagnostics_sequence: 10,
            unrecorded: 0,
        }
    }

    #[test]
    fn commands_requests_and_partial_spans_share_the_calibrated_clock() {
        let mut trace = Causal::default();
        trace.calibrate(&clock());
        let records = vec![
            record(
                11,
                11000,
                "span_open",
                Some(7),
                json!({"command_id":"demo.fetch","operation_id":"op-4"}),
            ),
            record(
                12,
                11500,
                "request",
                Some(7),
                json!({"phase":"started","request_id":3,"method":"GET","url":"http://fixture/","request_start_us":11500}),
            ),
            record(
                13,
                13000,
                "span_close",
                Some(7),
                json!({"command_id":"demo.fetch","operation_id":"op-4"}),
            ),
            record(
                14,
                14500,
                "request",
                Some(7),
                json!({"phase":"completed","request_id":3,"method":"GET","url":"http://fixture/","duration_us":3000}),
            ),
        ];
        let events = trace.observe(
            &Snapshot {
                records,
                dropped: 200,
                evicted: 200,
                output_dropped: 0,
            },
            &BTreeMap::new(),
        );
        let command = events
            .iter()
            .find(|event| {
                event["name"]
                    .as_str()
                    .is_some_and(|name| name.contains("demo.fetch"))
                    && event["ph"] == "b"
            })
            .unwrap();
        let request = events
            .iter()
            .find(|event| event["name"] == "HTTP GET http://fixture/" && event["ph"] == "b")
            .unwrap();
        assert_eq!(command["ts"], 2000);
        assert_eq!(request["ts"], 2500);
        assert_eq!(trace.cursor, 14);
        assert!(!trace.loss, "consumed eviction is not loss");
        let records = vec![record(15, 15000, "span_open", Some(8), json!({}))];
        trace.observe(
            &Snapshot {
                records,
                dropped: 200,
                evicted: 200,
                output_dropped: 0,
            },
            &BTreeMap::new(),
        );
        assert!(
            trace
                .finish(9000)
                .iter()
                .any(|event| event["name"].as_str().unwrap().starts_with("Incomplete /"))
        );
    }

    #[test]
    fn pending_retention_is_bounded_and_unread_gaps_report_loss() {
        let mut trace = Causal::default();
        trace.calibrate(&clock());
        let records = (11..300)
            .map(|id| {
                record(
                    id,
                    12000,
                    "span_open",
                    Some(id),
                    json!({"command_id":"x".repeat(256)}),
                )
            })
            .collect();
        trace.observe(
            &Snapshot {
                records,
                dropped: 0,
                evicted: 0,
                output_dropped: 0,
            },
            &BTreeMap::new(),
        );
        assert!(trace.spans.len() <= 128);
        assert!(trace.bytes <= 128 * 1024);
        assert!(trace.loss);
        let mut trace = Causal::default();
        trace.calibrate(&clock());
        trace.observe(
            &Snapshot {
                records: vec![record(13, 12000, "span_close", Some(1), json!({}))],
                dropped: 2,
                evicted: 2,
                output_dropped: 0,
            },
            &BTreeMap::new(),
        );
        assert!(trace.loss);
    }
}
