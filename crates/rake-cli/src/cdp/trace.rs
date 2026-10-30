//! Bounded Chrome trace-event export of retained spans, preserving missing observations.

use rake_diagnostics::{Record, Snapshot};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const MAX_BYTES: usize = 4 * 1024 * 1024;

pub(super) fn document(snapshot: &Snapshot) -> Value {
    let opens: BTreeMap<_, _> = snapshot
        .records
        .iter()
        .filter(|r| r.kind == "span_open")
        .filter_map(|r| r.span_id.map(|id| (id, r)))
        .collect();

    let closes: BTreeMap<_, _> = snapshot
        .records
        .iter()
        .filter(|r| r.kind == "span_close")
        .filter_map(|r| r.span_id.map(|id| (id, r)))
        .collect();

    let complete: Vec<_> = opens
        .iter()
        .filter_map(|(&id, &open)| {
            let close = closes.get(&id)?;

            Some((id, open, close.duration_us?))
        })
        .collect();

    let mut asynchronous = BTreeSet::new();

    for (i, &(id, a, ad)) in complete.iter().enumerate() {
        for &(other, b, bd) in &complete[i + 1..] {
            if a.elapsed_us < b.elapsed_us.saturating_add(bd)
                && b.elapsed_us < a.elapsed_us.saturating_add(ad)
                && !ancestor(id, b, &opens)
                && !ancestor(other, a, &opens)
            {
                asynchronous.insert(id);
                asynchronous.insert(other);
            }
        }
    }

    let mut events = Vec::new();
    let mut bytes = 512;
    let mut omitted = 0;

    for record in &snapshot.records {
        let Some(id) = record.span_id else {
            continue;
        };

        if !matches!(record.kind.as_str(), "span_open" | "span_close") {
            continue;
        }

        let complete =
            opens.contains_key(&id) && closes.get(&id).is_some_and(|r| r.duration_us.is_some());

        let mut event = json!({"name":record.spans.last().unwrap_or(&record.target),"cat":record.target,
            "pid":1,"tid":1,"ts":record.elapsed_us,"args":record.fields});

        event["args"]["span_id"] = json!(id);
        event["args"]["parent_span_id"] = json!(record.parent_span_id);

        if complete && asynchronous.contains(&id) {
            event["ph"] = json!(if record.kind == "span_open" { "b" } else { "e" });
            event["id"] = json!(id.to_string());

            // Close events share the measured open+duration timeline.
            if record.kind == "span_close" {
                event["ts"] = json!(
                    opens[&id]
                        .elapsed_us
                        .saturating_add(record.duration_us.unwrap_or_default())
                );
            }
        } else if complete {
            if record.kind == "span_close" {
                continue;
            }

            event["ph"] = json!("X");
            event["dur"] = json!(closes[&id].duration_us);
        } else {
            event["ph"] = json!("i");
            event["s"] = json!("t");
            event["args"]["incomplete"] = json!(true);
            event["args"]["missing"] = json!(if record.kind == "span_open" {
                "close"
            } else {
                "open"
            });
        }

        let size = event.to_string().len() + 1;

        if bytes + size > MAX_BYTES {
            omitted += 1;
            continue;
        }

        bytes += size;
        events.push(event);
    }

    events.sort_by_key(|event| event["ts"].as_u64().unwrap_or_default());

    json!({"traceEvents":events,"displayTimeUnit":"ms","metadata":{
        "clock":"runtime elapsed microseconds; logical thread, not CPU samples",
        "dropped":snapshot.dropped,"output_dropped":snapshot.output_dropped,"omitted":omitted}})
}

fn ancestor(id: u64, record: &Record, opens: &BTreeMap<u64, &Record>) -> bool {
    let mut parent = record.parent_span_id;

    // Bound traversal even for malformed/cyclic input.
    for _ in 0..opens.len() {
        let Some(current) = parent else {
            return false;
        };

        if current == id {
            return true;
        }

        parent = opens.get(&current).and_then(|r| r.parent_span_id);
    }

    false
}

pub(super) fn export(options: &super::Options, path: &std::path::Path) -> super::Result<()> {
    let instances = super::discover(&options.directory, options.selector.as_deref())?;

    let [instance] = instances.as_slice() else {
        return Err("trace export requires exactly one live instance; select --instance".into());
    };

    let rake_protocol::control::Reply::Diagnostics(snapshot) = crate::terminal::query(
        instance,
        rake_protocol::control::Query::Diagnostics {
            after: 0,
            kind: Some("span".into()),
        },
        super::TIMEOUT,
    )?
    else {
        return Err("unexpected trace diagnostics reply".into());
    };

    let bytes = serde_json::to_vec(&document(&snapshot))?;
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));

    let result = (|| -> super::Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;

        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)?;

        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }

    result?;

    if options.json {
        println!(
            "{}",
            json!({"event":"trace_exported","path":path,"bytes":bytes.len()})
        );
    } else {
        println!("Trace exported: {} ({} bytes)", path.display(), bytes.len());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(id: u64, parent: Option<u64>, start: u64, duration: Option<u64>, kind: &str) -> Record {
        serde_json::from_value(json!({"sequence":id,"elapsed_us":start,"level":"INFO","kind":kind,"target":"test",
            "spans":[format!("span-{id}")],"span_id":id,"parent_span_id":parent,"duration_us":duration,"fields":{}})).unwrap()
    }

    #[test]
    fn nesting_overlap_and_incomplete_spans() {
        let snapshot = Snapshot {
            records: vec![
                span(1, None, 0, None, "span_open"),
                span(2, Some(1), 10, None, "span_open"),
                span(2, Some(1), 20, Some(10), "span_close"),
                span(1, None, 50, Some(50), "span_close"),
                span(3, None, 100, None, "span_open"),
                span(4, None, 110, None, "span_open"),
                span(3, None, 130, Some(30), "span_close"),
                span(4, None, 140, Some(30), "span_close"),
                span(5, None, 150, None, "span_open"),
            ],
            dropped: 0,
            output_dropped: 0,
            evicted: 0,
        };

        let doc = document(&snapshot);
        let events = doc["traceEvents"].as_array().unwrap();

        assert_eq!(events.iter().filter(|e| e["ph"] == "X").count(), 2);
        assert_eq!(events.iter().filter(|e| e["ph"] == "b").count(), 2);
        assert_eq!(events.iter().filter(|e| e["ph"] == "e").count(), 2);
        assert_eq!(events.last().unwrap()["args"]["incomplete"], true);
        assert_eq!(events[1]["args"]["parent_span_id"], 1);
    }

    #[test]
    fn output_is_bounded_and_reports_omissions() {
        let mut record = span(1, None, 0, None, "span_open");

        record
            .fields
            .insert("large".into(), json!("x".repeat(MAX_BYTES)));
        let doc = document(&Snapshot {
            records: vec![record],
            dropped: 3,
            output_dropped: 1,
            evicted: 0,
        });

        assert!(doc.to_string().len() < MAX_BYTES);
        assert_eq!(doc["metadata"]["omitted"], 1);
        assert_eq!(doc["metadata"]["dropped"], 3);
    }
}
