//! Covers: bounded native output without blocking producers.
//! Real: diagnostic layer and bounded channel.
//! Doubles: an intentionally undrained output queue.
//! Requires: no terminal or external resources.

use tracing_subscriber::prelude::*;

use super::{Collector, OutputLayer};

#[test]
fn a_full_output_queue_reports_loss_and_keeps_collecting() {
    let collector = Collector::new(10);
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);

    let subscriber = tracing_subscriber::registry()
        .with(collector.layer())
        .with(OutputLayer {
            sender,
            collector: collector.clone(),
        });

    tracing::subscriber::with_default(subscriber, || {
        tracing::info!("first");
        tracing::info!("second");
    });
    let snapshot = collector.snapshot();

    assert_eq!(snapshot.records.len(), 2);
    assert_eq!(snapshot.output_dropped, 1);
    assert_eq!(receiver.try_iter().count(), 1);
}
