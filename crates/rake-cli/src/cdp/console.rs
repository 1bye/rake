//! Console formatting and bounded JSON object expansion; no runtime code executes.

use super::wire::{Error, Event};
use rake_diagnostics::{Record, Snapshot};
use serde_json::{Value, json};
use std::collections::VecDeque;

pub(super) fn level(level: &str) -> &'static str {
    match level {
        "TRACE" | "DEBUG" => "verbose",
        "WARN" => "warning",
        "ERROR" => "error",
        _ => "info",
    }
}

fn display(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

fn short(value: &str) -> String {
    value.chars().take(160).collect()
}

pub(super) fn entry(level: &str, text: String, timestamp_ms: f64, url: &str) -> Event {
    Event::new(
        "Log.entryAdded",
        json!({"entry":{"source":"other", "level":level,
        "text":text,"timestamp":timestamp_ms,"url":url}}),
    )
}

pub(super) fn translate(record: &Record, epoch: f64) -> Option<Event> {
    if record.kind != "event" {
        return None;
    }

    let mut text = record
        .fields
        .get("message")
        .map(display)
        .unwrap_or_default();

    for (key, value) in &record.fields {
        if key == "message" {
            continue;
        }

        if !text.is_empty() {
            text.push(' ');
        }

        text.push_str(&format!("{key}={}", display(value)));
    }

    let mut event = entry(
        level(&record.level),
        text,
        (epoch + record.elapsed_us as f64 / 1e6) * 1000.,
        &record.target,
    );
    if let Some(source) = &record.source {
        event.params["entry"]["url"] = json!(source_url(source));
        event.params["entry"]["lineNumber"] = json!(source.line.saturating_sub(1));
    }
    Some(event)
}

#[derive(Default)]
pub(super) struct Objects {
    values: VecDeque<(String, Value, usize, Option<String>)>,
    next: u64,
    bytes: usize,
}

impl Objects {
    pub fn remote_in(&mut self, value: Value, group: Option<&str>) -> Value {
        let kind = match &value {
            Value::Null => "object",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            _ => "object",
        };

        if !value.is_object() && !value.is_array() {
            let mut result = json!({"type":kind,"value":value});

            if value.is_null() {
                result["subtype"] = json!("null");
            }

            return result;
        }

        self.next += 1;
        let id = format!("rake-object-{}", self.next);

        let properties: Vec<_> = match &value {
            Value::Object(fields) => fields.iter().take(5).map(|(k, v)| preview(k, v)).collect(),
            Value::Array(values) => values
                .iter()
                .take(5)
                .enumerate()
                .map(|(i, v)| preview(&i.to_string(), v))
                .collect(),
            _ => Vec::new(),
        };

        let length = value
            .as_object()
            .map(|v| v.len())
            .or_else(|| value.as_array().map(|v| v.len()))
            .unwrap_or(0);

        let mut result = json!({"type":"object","objectId":id,"className":"Object","description":"Object",
            "preview":{"type":"object","description":"Object","overflow":length>5,"properties":properties}});

        if value.is_array() {
            result["subtype"] = json!("array");
            result["className"] = json!("Array");
            result["description"] = json!(format!("Array({length})"));
            result["preview"]["subtype"] = json!("array");
        }

        let bytes = value.to_string().len();

        self.bytes += bytes;
        self.values
            .push_back((id, value, bytes, group.map(str::to_owned)));

        while self.values.len() > 128 || self.bytes > 1024 * 1024 {
            if let Some((_, _, bytes, _)) = self.values.pop_front() {
                self.bytes -= bytes;
            }
        }

        result
    }

    pub fn properties(&mut self, id: &str) -> Result<Value, Error> {
        let (value, group) = self
            .values
            .iter()
            .find(|(key, _, _, _)| key == id)
            .map(|(_, v, _, group)| (v.clone(), group.clone()))
            .ok_or_else(|| Error::new(-32000, "Object expired or belongs to another session"))?;

        let fields: Vec<(String, Value)> = match value {
            Value::Object(fields) => fields.into_iter().collect(),
            Value::Array(values) => values
                .into_iter()
                .enumerate()
                .map(|(i, v)| (i.to_string(), v))
                .collect(),
            _ => Vec::new(),
        };

        let properties: Vec<_> = fields
            .into_iter()
            .map(|(name, value)| {
                json!({"name":name,"value":self.remote_in(value, group.as_deref()),
            "writable":false,"configurable":false,"enumerable":true,"isOwn":true})
            })
            .collect();

        Ok(json!({"result":properties,"internalProperties":[]}))
    }

    pub fn release(&mut self, id: &str) {
        self.values.retain(|(key, _, _, _)| key != id);
        self.bytes = self.values.iter().map(|(_, _, n, _)| n).sum();
    }

    pub fn release_group(&mut self, group: &str) {
        self.values
            .retain(|(_, _, _, owner)| owner.as_deref() != Some(group));
        self.bytes = self.values.iter().map(|(_, _, bytes, _)| bytes).sum();
    }

    pub fn clear(&mut self) {
        self.values.clear();
        self.bytes = 0;
    }

    pub fn logged(&mut self, record: &Record, epoch: f64) -> Event {
        let called = self.called(record, epoch);
        let mut event = translate(record, epoch).expect("event records only");
        event.params["entry"]["text"] = called.params["args"][0]["value"].clone();
        event.params["entry"]["args"] = json!([called.params["args"][1]]);
        event
    }

    pub fn called(&mut self, record: &Record, epoch: f64) -> Event {
        let fields = json!({"fields":record.fields,"target":record.target,"spans":record.spans,
            "spanId":record.span_id,"parentSpanId":record.parent_span_id,"sequence":record.sequence,
            "source":record.source});

        let kind = match level(&record.level) {
            "verbose" => "debug",
            "warning" => "warning",
            "error" => "error",
            _ => "log",
        };

        let message = record
            .fields
            .get("message")
            .map(display)
            .unwrap_or_else(|| record.target.clone());

        let mut params = json!({"type":kind,"args":[self.remote_in(json!(message), Some("console")),self.remote_in(fields, Some("console"))],
            "executionContextId":1,"timestamp":(epoch+record.elapsed_us as f64/1e6)*1000.});
        if let Some(source) = &record.source {
            params["stackTrace"] = json!({"callFrames":[{"functionName":record.target,"scriptId":"",
                "url":source_url(source),"lineNumber":source.line.saturating_sub(1),"columnNumber":0}]});
        }
        Event::new("Runtime.consoleAPICalled", params)
    }
}

fn preview(name: &str, value: &Value) -> Value {
    let kind = match value {
        Value::Null | Value::Object(_) | Value::Array(_) => "object",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
    };

    json!({"name":name,"type":kind,"value":short(&display(value))})
}

#[derive(Default)]
pub(super) struct Loss {
    dropped: u64,
    output_dropped: u64,
}

impl Loss {
    pub fn observe(&mut self, snapshot: &Snapshot, cursor: u64) -> Option<String> {
        let mut previous = cursor;
        let mut skipped = 0;

        for record in &snapshot.records {
            skipped += record.sequence.saturating_sub(previous.saturating_add(1));
            previous = record.sequence;
        }

        let changed = snapshot.dropped > self.dropped
            || snapshot.output_dropped > self.output_dropped
            || skipped > 0;

        self.dropped = snapshot.dropped;
        self.output_dropped = snapshot.output_dropped;

        changed.then(|| {
            format!(
                "Rake diagnostics loss: dropped={}, output_dropped={}, skipped={skipped}",
                snapshot.dropped, snapshot.output_dropped
            )
        })
    }
}

pub(super) struct ProcessOutput {
    path: Option<std::path::PathBuf>,
    cursor: u64,
    epoch: Option<f64>,
    dropped: u64,
    last_error: Option<String>,
}

impl ProcessOutput {
    pub fn new(path: Option<std::path::PathBuf>) -> Self {
        Self {
            path,
            cursor: 0,
            epoch: None,
            dropped: 0,
            last_error: None,
        }
    }

    fn read(&self) -> super::Result<Option<Value>> {
        use std::io::Read;
        use std::os::unix::fs::MetadataExt;

        let Some(path) = &self.path else {
            return Ok(None);
        };

        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };

        let metadata = file.metadata()?;

        if !metadata.is_file()
            || metadata.mode() & 0o077 != 0
            || metadata.uid()
                != std::fs::metadata(path.parent().ok_or("output parent missing")?)?.uid()
        {
            return Err("process output must be a private owned regular file".into());
        }

        let mut bytes = Vec::new();

        file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;

        if bytes.len() > 1024 * 1024 {
            return Err("process output exceeds 1 MiB".into());
        }

        Ok(Some(serde_json::from_slice(&bytes)?))
    }

    pub fn poll(&mut self, now: f64, include_mirrors: bool) -> Vec<Event> {
        let snapshot = match self.read() {
            Ok(Some(snapshot)) => {
                self.last_error = None;

                snapshot
            }
            Ok(None) => return Vec::new(),
            Err(error) => {
                let text = error.to_string();

                if self.last_error.as_ref() == Some(&text) {
                    return Vec::new();
                }

                self.last_error = Some(text.clone());

                return vec![entry(
                    "warning",
                    format!("Process output unavailable: {text}"),
                    now * 1000.,
                    "rake://process",
                )];
            }
        };

        let mut events = Vec::new();
        let records = snapshot["records"].as_array().cloned().unwrap_or_default();

        if let Some(last) = records.last() {
            self.epoch
                .get_or_insert(now - last["elapsed_us"].as_f64().unwrap_or_default() / 1e6);
        }

        let dropped = snapshot["dropped"].as_u64().unwrap_or_default();
        let mut skipped = 0;

        for record in records {
            let sequence = record["sequence"].as_u64().unwrap_or_default();

            if sequence <= self.cursor {
                continue;
            }

            skipped += sequence.saturating_sub(self.cursor + 1);
            self.cursor = sequence;
            if !include_mirrors && record["structured"] == true {
                continue;
            }
            let stream = record["stream"].as_str().unwrap_or("output");
            let text = record["text"].as_str().unwrap_or_default();

            let severity = match record["level"]
                .as_str()
                .or_else(|| text.split_whitespace().next())
            {
                Some("info" | "INFO") => "info",
                Some("warning" | "WARN") => "warning",
                Some("verbose" | "DEBUG" | "TRACE") => "verbose",
                Some("error" | "ERROR") => "error",
                _ if stream == "stderr" => "error",
                _ => "info",
            };
            events.push(entry(
                severity,
                format!("[{stream}] {text}"),
                (self.epoch.unwrap_or(now)
                    + record["elapsed_us"].as_f64().unwrap_or_default() / 1e6)
                    * 1000.,
                "rake://process",
            ));
        }

        if dropped > self.dropped || skipped > 0 {
            events.push(entry(
                "warning",
                format!("Process output loss: dropped={dropped}, skipped={skipped}"),
                now * 1000.,
                "rake://process",
            ));
        }

        self.dropped = dropped;

        events
    }
}

pub(super) fn source_url(source: &rake_diagnostics::Source) -> String {
    // Keep the advertised URL opaque: only captured paths may be read by Sources.
    format!("rake://source/{}", source.file)
}

pub(super) fn exception(error: Error, sequence: u64) -> Value {
    let description = format!("RakeError ({}): {}", error.code, error.message);
    let remote = json!({"type":"object","subtype":"error","className":"RakeError","description":description});
    json!({"result":remote,"exceptionDetails":{"exceptionId":sequence,"text":"Uncaught",
        "lineNumber":0,"columnNumber":0,"exception":remote}})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn releasing_groups_expires_nested_handles_without_touching_other_results() {
        let mut objects = Objects::default();
        let first = objects.remote_in(json!({"nested":{"text":"hello"}}), Some("console"));
        let other = objects.remote_in(json!({"kept":true}), Some("watch"));
        let first_id = first["objectId"].as_str().unwrap();
        let nested = objects.properties(first_id).unwrap()["result"][0]["value"]["objectId"]
            .as_str()
            .unwrap()
            .to_owned();
        objects.release_group("console");
        assert!(objects.properties(first_id).is_err());
        assert!(objects.properties(&nested).is_err());
        assert!(
            objects
                .properties(other["objectId"].as_str().unwrap())
                .is_ok()
        );
        objects.clear();
        let fresh = objects.remote_in(json!({"fresh":true}), None);
        assert_ne!(
            fresh["objectId"], first["objectId"],
            "expired handles must never retarget"
        );
    }

    #[test]
    fn levels_preview_expansion_and_eviction() {
        for (input, output) in [
            ("TRACE", "verbose"),
            ("DEBUG", "verbose"),
            ("INFO", "info"),
            ("WARN", "warning"),
            ("ERROR", "error"),
        ] {
            assert_eq!(level(input), output);
        }

        let mut objects = Objects::default();
        let remote = objects.remote_in(json!({"count":3,"nested":{"yes":true}}), None);

        assert_eq!(remote["preview"]["properties"][0]["name"], "count");
        assert_eq!(
            objects
                .properties(remote["objectId"].as_str().unwrap())
                .unwrap()["result"][0]["value"]["value"],
            3
        );

        for _ in 0..128 {
            objects.remote_in(json!({"next":true}), None);
        }

        assert!(
            objects
                .properties(remote["objectId"].as_str().unwrap())
                .is_err()
        );
    }

    #[test]
    fn cumulative_loss_warns_only_on_changes() {
        let mut loss = Loss::default();

        let mut snapshot = Snapshot {
            records: vec![],
            dropped: 2,
            output_dropped: 1,
        };

        assert!(loss.observe(&snapshot, 0).unwrap().contains("dropped=2"));
        assert!(loss.observe(&snapshot, 0).is_none());
        snapshot.records.push(serde_json::from_value(json!({"sequence":4,"elapsed_us":0,"level":"INFO","target":"test","kind":"event","spans":[],"fields":{}})).unwrap());
        assert!(loss.observe(&snapshot, 1).unwrap().contains("skipped=2"));
    }

    #[test]
    fn marked_mirrors_are_hidden_live_but_retained_after_runtime_failure() {
        use std::os::unix::fs::OpenOptionsExt;
        let path = std::env::temp_dir().join(format!("rake-mirror-{}.json", uuid::Uuid::new_v4()));
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        serde_json::to_writer(file,&json!({"records":[{"sequence":1,"elapsed_us":0,"stream":"stderr","text":"INFO app mirrored","structured":true},
            {"sequence":2,"elapsed_us":1,"stream":"stderr","text":"panic output","structured":false}],"dropped":0})).unwrap();
        let mut live = ProcessOutput::new(Some(path.clone()));
        let live_entries = live.poll(10., false);
        assert_eq!(live_entries.len(), 1);
        assert_eq!(
            live_entries[0].params["entry"]["text"],
            "[stderr] panic output"
        );
        assert!(live.poll(11., false).is_empty());
        let mut disconnected = ProcessOutput::new(Some(path.clone()));
        assert_eq!(disconnected.poll(10., true).len(), 2);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn process_output_has_its_own_clock_cursor_and_loss() {
        use std::os::unix::fs::OpenOptionsExt;
        let path = std::env::temp_dir().join(format!("rake-output-{}.json", uuid::Uuid::new_v4()));

        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();

        serde_json::to_writer(file,&json!({"dropped":1,"records":[{"sequence":2,"elapsed_us":500000,"stream":"stderr","text":"test"}]})).unwrap();
        let mut output = ProcessOutput::new(Some(path.clone()));
        let events = output.poll(10., true);

        assert_eq!(events[0].params["entry"]["text"], "[stderr] test");
        assert_eq!(events[0].params["entry"]["level"], "error");
        assert_eq!(events[0].params["entry"]["timestamp"], 10000.);
        assert!(
            events[1].params["entry"]["text"]
                .as_str()
                .unwrap()
                .contains("skipped=1")
        );
        assert!(output.poll(20., true).is_empty());
        std::fs::remove_file(path).unwrap();
    }
}
