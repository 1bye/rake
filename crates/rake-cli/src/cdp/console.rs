//! Console formatting and bounded JSON object expansion; no runtime code executes.

use super::wire::{Error, Event};
use rake::diagnostics::{Record, Snapshot};
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

    Some(entry(
        level(&record.level),
        text,
        (epoch + record.elapsed_us as f64 / 1e6) * 1000.,
        &record.target,
    ))
}

#[derive(Default)]
pub(super) struct Objects {
    values: VecDeque<(String, Value, usize)>,
    next: u64,
    bytes: usize,
}

impl Objects {
    pub fn remote(&mut self, value: Value) -> Value {
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
        self.values.push_back((id, value, bytes));

        while self.values.len() > 128 || self.bytes > 1024 * 1024 {
            if let Some((_, _, bytes)) = self.values.pop_front() {
                self.bytes -= bytes;
            }
        }

        result
    }

    pub fn properties(&mut self, id: &str) -> Result<Value, Error> {
        let value = self
            .values
            .iter()
            .find(|(key, _, _)| key == id)
            .map(|(_, v, _)| v.clone())
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
                json!({"name":name,"value":self.remote(value),
            "writable":false,"configurable":false,"enumerable":true,"isOwn":true})
            })
            .collect();

        Ok(json!({"result":properties,"internalProperties":[]}))
    }

    pub fn release(&mut self, id: &str) {
        self.values.retain(|(key, _, _)| key != id);
        self.bytes = self.values.iter().map(|(_, _, n)| n).sum();
    }

    pub fn clear(&mut self) {
        self.values.clear();
        self.bytes = 0;
    }

    pub fn called(&mut self, record: &Record, epoch: f64) -> Event {
        let fields = serde_json::to_value(&record.fields).unwrap_or_default();

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

        Event::new(
            "Runtime.consoleAPICalled",
            json!({"type":kind,"args":[self.remote(json!(message)),self.remote(fields)],
            "executionContextId":1,"timestamp":(epoch+record.elapsed_us as f64/1e6)*1000.}),
        )
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

    pub fn poll(&mut self, now: f64) -> Vec<Event> {
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
            let stream = record["stream"].as_str().unwrap_or("output");
            let text = record["text"].as_str().unwrap_or_default();

            events.push(entry(
                "info",
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let remote = objects.remote(json!({"count":3,"nested":{"yes":true}}));

        assert_eq!(remote["preview"]["properties"][0]["name"], "count");
        assert_eq!(
            objects
                .properties(remote["objectId"].as_str().unwrap())
                .unwrap()["result"][0]["value"]["value"],
            3
        );

        for _ in 0..128 {
            objects.remote(json!({"next":true}));
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
        let events = output.poll(10.);

        assert_eq!(events[0].params["entry"]["text"], "[stderr] test");
        assert_eq!(events[0].params["entry"]["timestamp"], 10000.);
        assert!(
            events[1].params["entry"]["text"]
                .as_str()
                .unwrap()
                .contains("skipped=1")
        );
        assert!(output.poll(20.).is_empty());
        std::fs::remove_file(path).unwrap();
    }
}
