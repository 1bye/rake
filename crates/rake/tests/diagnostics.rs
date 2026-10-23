//! Covers: typed logs, macro spans, bounded overflow, and host subscriber integration.
//! Real: tracing subscriber, Rake macros and collector.
//! Doubles: none.
//! Requires: no external resources.

use rake::diagnostics::{Collector, DiagnosticsConfig, SubscriberMode};
use tracing_subscriber::prelude::*;

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
