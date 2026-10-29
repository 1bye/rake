//! Covers: filtered incremental snapshots, owned data and lifetime loss counters.
//! Real: standalone collector and its bounded record storage.
//! Doubles: none. Requires: no UI, HTTP runtime or external services.

use std::collections::BTreeMap;

use rake_diagnostics::Collector;
use serde_json::json;

#[test]
fn incremental_snapshots_filter_before_copying_and_keep_global_loss_counts() {
    let collector = Collector::new(3);

    for (index, kind) in ["event", "span_open", "request", "span_close", "event"]
        .into_iter()
        .enumerate()
    {
        collector.record(
            "INFO",
            "test",
            kind,
            Vec::new(),
            BTreeMap::from([("index".into(), json!(index))]),
        );
    }

    for (after, kind, expected) in [
        (0, None, vec![3, 4, 5]),
        (3, None, vec![4, 5]),
        (0, Some("request"), vec![3]),
        (3, Some("span"), vec![4]),
        (0, Some("span_open"), vec![]),
        (5, None, vec![]),
        (u64::MAX, None, vec![]),
        (0, Some("unknown"), vec![]),
    ] {
        let snapshot = collector.snapshot_after(after, kind);

        let sequences: Vec<_> = snapshot
            .records
            .iter()
            .map(|record| record.sequence)
            .collect();

        assert_eq!(sequences, expected, "after={after}, kind={kind:?}");
        assert_eq!(snapshot.dropped, 2);
        assert_eq!(snapshot.output_dropped, 0);
    }

    let mut snapshot = collector.snapshot_after(4, Some("event"));

    snapshot.records[0].fields.insert("index".into(), json!(99));
    assert_eq!(collector.snapshot().records[2].fields["index"], 4);
}

#[test]
fn span_alias_includes_both_lifecycle_records_with_an_exclusive_cursor() {
    let collector = Collector::new(8);

    for kind in ["span_open", "event", "span_close"] {
        collector.record("INFO", "test", kind, Vec::new(), BTreeMap::new());
    }

    let snapshot = collector.snapshot_after(0, Some("span"));

    assert_eq!(snapshot.records.len(), 2);
    assert_eq!(snapshot.records[0].kind, "span_open");
    assert_eq!(snapshot.records[1].kind, "span_close");
    assert_eq!(collector.snapshot_after(1, Some("span")).records.len(), 1);
    assert_eq!(collector.snapshot_after(0, None).records.len(), 3);
}
