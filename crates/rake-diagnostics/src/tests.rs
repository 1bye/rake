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
            #[cfg(feature = "devtools")]
            mirror_marker: false,
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

#[test]
fn crash_tail_skips_locked_and_poisoned_buffers() {
    let collector = Collector::new(8);
    let guard = collector.0.buffer.lock().expect("buffer lock");

    assert!(collector.try_crash_tail(8).is_none());
    drop(guard);
    let copy = collector.clone();

    let _ = std::thread::spawn(move || {
        let _guard = copy.0.buffer.lock().expect("buffer lock");

        panic!("poison test buffer");
    })
    .join();

    assert!(collector.try_crash_tail(8).is_none());
}

#[test]
fn crash_tail_keeps_only_bounded_structural_metadata() {
    let collector = Collector::new(200);

    for _ in 0..150 {
        collector.record(
            "SECRET",
            "private/path",
            "private kind",
            vec!["private span".into()],
            [("password".into(), serde_json::json!("secret"))].into(),
        );
    }

    let tail = collector
        .try_crash_tail(usize::MAX)
        .expect("available buffer");

    assert_eq!(tail.len(), 128);
    assert_eq!(tail[0].sequence, 23);
    let json = serde_json::to_string(&tail).expect("tail JSON");

    assert!(!json.contains("private"));
    assert!(!json.contains("secret"));
    assert!(
        tail.iter()
            .all(|item| item.level == "unknown" && item.kind == "unknown")
    );
}
