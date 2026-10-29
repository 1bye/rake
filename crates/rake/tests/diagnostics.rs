//! Covers: typed logs, macro spans, bounded overflow, and host subscriber integration.
//! Real: tracing subscriber, Rake macros and collector.
//! Doubles: none.
//! Requires: no external resources.

use rake::diagnostics::{Collector, DiagnosticsConfig, SubscriberMode};
use tracing_subscriber::prelude::*;

#[test]
fn facade_reexports_the_standalone_collector_without_a_wrapper() {
    let collector: rake_diagnostics::Collector = Collector::new(2);

    let config: rake_diagnostics::DiagnosticsConfig = DiagnosticsConfig::new()
        .collector(collector.clone())
        .subscriber(SubscriberMode::Host);

    config.install().expect("host-owned subscriber");
    collector.record("INFO", "facade", "event", Vec::new(), Default::default());
    let snapshot = config.shared_collector().snapshot_after(0, None);

    assert_eq!(snapshot.records.len(), 1);
    assert_eq!(snapshot.records[0].target, "facade");
}

#[rake::instrument(skip_all)]
async fn suspended() {
    futures::future::pending::<()>().await;
}

#[rake::instrument(skip_all, fields(document = %id))]
fn load(id: u64) -> u64 {
    rake::info!(count = 3_u64, saved = true, "Loaded");

    id
}

#[test]
fn a_host_owned_subscriber_receives_typed_fields_and_instrumentation() {
    let collector = Collector::new(20);
    let subscriber = tracing_subscriber::registry().with(collector.layer());

    let config = DiagnosticsConfig::new()
        .collector(collector.clone())
        .subscriber(SubscriberMode::Host);

    config.install().expect("host integration");

    tracing::subscriber::with_default(subscriber, || assert_eq!(load(42), 42));
    let snapshot = collector.snapshot();

    let event = snapshot
        .records
        .iter()
        .find(|record| record.kind == "event")
        .expect("event");

    assert_eq!(event.fields["count"], 3);
    assert_eq!(event.fields["saved"], true);
    assert_eq!(event.spans, ["load"]);
    assert!(
        snapshot
            .records
            .iter()
            .any(|record| record.kind == "span_open" && record.fields["document"] == "42")
    );
    assert!(
        snapshot
            .records
            .iter()
            .any(|record| record.kind == "span_close")
    );
}

#[test]
fn overflow_is_bounded_and_visible() {
    let collector = Collector::new(2);
    let subscriber = tracing_subscriber::registry().with(collector.layer());

    tracing::subscriber::with_default(subscriber, || {
        for index in 0..5 {
            rake::info!(index, "event");
        }
    });
    let snapshot = collector.snapshot();

    assert_eq!(snapshot.records.len(), 2);
    assert_eq!(snapshot.dropped, 3);
    assert_eq!(snapshot.records[0].fields["index"], 3);
    assert_eq!(snapshot.records[1].fields["index"], 4);
}

#[test]
fn async_instrumentation_does_not_enter_a_span_across_suspension() {
    use std::future::Future;
    use std::task::{Context, Poll};

    let collector = Collector::new(20);
    let subscriber = tracing_subscriber::registry().with(collector.layer());

    tracing::subscriber::with_default(subscriber, || {
        let mut future = Box::pin(suspended());
        let mut context = Context::from_waker(futures::task::noop_waker_ref());

        assert_eq!(future.as_mut().poll(&mut context), Poll::Pending);
        rake::info!("outside the suspended future");
    });
    let snapshot = collector.snapshot();

    let outside = snapshot
        .records
        .iter()
        .find(|record| record.kind == "event")
        .expect("outside event");

    assert!(outside.spans.is_empty());
}

#[test]
fn repeated_span_names_have_distinct_ids_parentage_fields_updates_and_elapsed_times() {
    let collector = Collector::new(50);
    let subscriber = tracing_subscriber::registry().with(collector.layer());

    tracing::subscriber::with_default(subscriber, || {
        let root = tracing::info_span!("root", operation_id = "op-9");
        let _root = root.enter();

        let first =
            tracing::info_span!("same", task_id = "task-1", progress = tracing::field::Empty);

        first.record("progress", 3u64);
        first.in_scope(|| rake::info!("first"));
        let second = tracing::info_span!("same", task_id = "task-2");

        second.in_scope(|| rake::info!("second"));
    });
    let snapshot = collector.snapshot();

    let opens: Vec<_> = snapshot
        .records
        .iter()
        .filter(|record| record.kind == "span_open")
        .collect();

    assert_ne!(opens[1].span_id, opens[2].span_id);
    assert_eq!(opens[1].parent_span_id, opens[0].span_id);
    assert_eq!(opens[2].parent_span_id, opens[0].span_id);
    let event = snapshot
        .records
        .iter()
        .find(|record| {
            record
                .fields
                .get("message")
                .and_then(|value| value.as_str())
                == Some("first")
        })
        .unwrap();

    assert_eq!(event.fields["operation_id"], "op-9");
    assert_eq!(event.fields["task_id"], "task-1");
    assert_eq!(event.fields["progress"], 3);
    assert_eq!(event.span_id, opens[1].span_id);
    assert!(
        snapshot
            .records
            .iter()
            .filter(|record| record.kind == "span_close")
            .all(|record| record.duration_us.is_some())
    );
}

#[test]
fn independent_collectors_can_share_a_subscriber_without_span_extension_collisions() {
    let a = Collector::new(10);
    let b = Collector::new(10);

    let subscriber = tracing_subscriber::registry()
        .with(a.layer())
        .with(b.layer());

    tracing::subscriber::with_default(subscriber, || {
        let span = tracing::info_span!("shared");

        span.in_scope(|| rake::info!("shared event"));
    });
    assert_eq!(a.snapshot().records.len(), 3);
    assert_eq!(b.snapshot().records.len(), 3);
}
