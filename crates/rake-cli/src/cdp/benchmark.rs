//! Covers: repeatable tree projection latency and allocation baselines.
//! Real: DOM projection and allocator. Doubles: generated inspection captures.
//! Requires: explicitly run ignored tests with one test thread, in release mode.

use super::dom::Dom;
use rake_protocol::inspection::{Element, Snapshot, Status};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::time::Instant;

struct CountingAllocator;

thread_local! {
    static COUNT: Cell<Option<(usize, usize)>> = const { Cell::new(None) };
}

// SAFETY: allocations retain System's layout and ownership contract; the counter
// is allocation-free thread-local bookkeeping and never inspects allocated data.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = COUNT.try_with(|count| {
            if let Some((allocations, bytes)) = count.get() {
                count.set(Some((allocations + 1, bytes + layout.size())));
            }
        });
        // SAFETY: the caller supplies the GlobalAlloc layout contract unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the allocation originated from System with this same layout.
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

pub(super) fn capture(count: usize) -> Snapshot {
    let template: Element = serde_json::from_value(serde_json::json!({
        "id":"root", "kind":"Div", "metadata_truncated":false
    }))
    .unwrap();
    Snapshot {
        status: Status::Captured,
        capture: "one".into(),
        router_revision: 0,
        omitted: 0,
        coverage: "generated benchmark boundaries".into(),
        elements: (0..count)
            .map(|index| Element {
                id: format!("element-one-{index}"),
                stable_id: Some(format!("boundary-{index}")),
                parent: (index > 0).then(|| "element-one-0".into()),
                text: Some(format!("Row {index}")),
                ..template.clone()
            })
            .collect(),
    }
}

pub(super) fn measure<T>(operation: impl FnOnce() -> T) -> (T, u128, usize, usize) {
    COUNT.set(Some((0, 0)));
    let start = Instant::now();
    let result = operation();
    let micros = start.elapsed().as_micros();
    let (allocations, bytes) = COUNT.replace(None).unwrap();
    (result, micros, allocations, bytes)
}

#[test]
#[ignore = "explicit release-mode performance baseline"]
fn cdp_projection_baseline() {
    for count in [1_000, 8_192] {
        let snapshot = capture(count);
        let mut dom = Dom::default();
        dom.update_index(snapshot.clone()).unwrap();
        let mut times = Vec::with_capacity(25);
        let mut allocations = 0;
        let mut allocated_bytes = 0;
        for _ in 0..25 {
            let input = snapshot.clone();
            COUNT.set(Some((0, 0)));
            let start = Instant::now();
            dom.update_index(input).unwrap();
            let elapsed = start.elapsed().as_micros();
            let (calls, bytes) = COUNT.replace(None).unwrap();
            times.push(elapsed);
            allocations += calls;
            allocated_bytes += bytes;
        }
        times.sort_unstable();
        println!(
            "cdp_projection nodes={count} median_us={} p95_us={} allocations={} allocated_bytes={}",
            times[12],
            times[23],
            allocations / 25,
            allocated_bytes / 25
        );
    }
}
