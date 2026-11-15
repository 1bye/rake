//! Covers: bounded native output without blocking producers.
//! Real: diagnostic layer and bounded channel.
//! Doubles: an intentionally undrained output queue.
//! Requires: no terminal or external resources.

use tracing_subscriber::prelude::*;

use super::{Collector, DiagnosticsConfig, Instant, OutputLayer, Registry};

#[test]
fn a_full_output_queue_reports_loss_and_keeps_collecting() {
    let collector = Collector::new(10);
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);

    let subscriber = tracing_subscriber::registry()
        .with(collector.layer())
        .with(OutputLayer {
            private: false,
            started: Instant::now(),
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

#[test]
fn private_output_is_bounded_concurrent_and_omits_payloads() {
    use std::io::{self, Write};
    use std::sync::{Mutex, mpsc};
    use std::time::Duration;

    struct Lines(mpsc::Sender<Vec<u8>>);

    impl Write for Lines {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.send(bytes.to_vec()).map_err(io::Error::other)?;

            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let (sender, receiver) = mpsc::channel();
    let sender = Mutex::new(sender);
    let config = DiagnosticsConfig::new()
        .log_writer(move || Box::new(Lines(sender.lock().expect("test sender").clone())));
    let collector = config.shared_collector();
    let subscriber =
        Registry::default().with(OutputLayer::new(collector.clone(), &config).expect("worker"));
    let dispatch = tracing::Dispatch::new(subscriber);

    std::thread::scope(|scope| {
        for _ in 0..4 {
            let dispatch = dispatch.clone();

            scope.spawn(move || {
                tracing::dispatcher::with_default(&dispatch, || {
                    for _ in 0..20 {
                        tracing::warn!(token = "secret-token", "private user content");
                    }
                })
            });
        }
    });

    drop(config);
    drop(dispatch);

    let mut output = Vec::new();

    while let Ok(bytes) = receiver.recv_timeout(Duration::from_secs(3)) {
        output.extend(bytes);
    }

    let output = String::from_utf8(output).expect("UTF-8 output");

    assert_eq!(output.lines().count(), 80);
    assert!(output.lines().all(|line| line.ends_with(" WARN event")));
    assert!(!output.contains("secret") && !output.contains("private user content"));
    assert_eq!(collector.snapshot().output_dropped, 0);
}

#[test]
fn full_output_queue_drops_without_waiting() {
    let (sender, _receiver) = std::sync::mpsc::sync_channel(1);
    sender.try_send("occupied".into()).expect("fill queue");
    let collector = Collector::default();
    let layer = OutputLayer {
        private: true,
        started: Instant::now(),
        sender,
        collector: collector.clone(),
        #[cfg(feature = "devtools")]
        mirror_marker: false,
    };
    let subscriber = Registry::default().with(layer);
    tracing::subscriber::with_default(subscriber, || tracing::info!("dropped"));

    assert_eq!(collector.snapshot().output_dropped, 1);
}

#[test]
fn log_tail_preserves_loss_counts_and_cursor_after_eviction() {
    let collector = Collector::new(1);

    for _ in 0..2 {
        collector.record(
            "INFO",
            "private-target",
            "event",
            Vec::new(),
            Default::default(),
        );
    }

    let snapshot = collector.logs_after(0);

    assert_eq!(snapshot.records.len(), 1);
    assert_eq!(snapshot.dropped, 1);

    let next = collector.logs_after(snapshot.records[0].sequence);

    assert!(next.records.is_empty());
    assert_eq!(next.dropped, 1);
    assert!(
        !serde_json::to_string(&snapshot)
            .expect("log JSON")
            .contains("private")
    );
}
