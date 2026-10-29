//! Measures owned diagnostic snapshots for initial and incremental inspector polling.

use std::hint::black_box;
use std::time::Instant;

use rake_diagnostics::{Collector, Snapshot};
use tracing_subscriber::prelude::*;

fn poll(collector: &Collector, after: u64) -> Snapshot {
    collector.snapshot_after(after, Some("event"))
}

fn main() {
    let collector = Collector::new(1024);
    let subscriber = tracing_subscriber::registry().with(collector.layer());
    let payload = "x".repeat(256);

    tracing::subscriber::with_default(subscriber, || {
        for index in 0..1024_u64 {
            tracing::info!(index, payload, "snapshot fixture");
        }
    });

    for (name, after, expected) in [
        ("initial", 0, 1024),
        ("recent", 1014, 10),
        ("idle", 1024, 0),
    ] {
        assert_eq!(poll(&collector, after).records.len(), expected);

        for _ in 0..100 {
            black_box(poll(&collector, after));
        }

        let mut samples = Vec::new();

        for _ in 0..5 {
            let started = Instant::now();

            for _ in 0..500 {
                black_box(poll(black_box(&collector), black_box(after)));
            }

            samples.push(started.elapsed().as_nanos() / 500);
        }

        samples.sort_unstable();
        println!(
            "{name}: {} ns/poll; {expected} returned records",
            samples[2]
        );
    }
}
