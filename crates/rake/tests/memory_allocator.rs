//! Covers: public allocator installation and real control-protocol capture.
//! Real: System allocations, Unix stack unwinding, native symbolization and router host.
//! Double: none. Requires the explicit devtools-memory feature.

#![cfg(all(feature = "devtools-memory", unix))]

use rake::{
    control::{Command, Message, PROTOCOL_VERSION, Query, Reply, Request},
    router::{NoRoute, Router, RouterConfig},
};

#[global_allocator]
static ALLOCATOR: rake::memory::TrackingAllocator<std::alloc::System> =
    rake::memory::TrackingAllocator::new(std::alloc::System);

#[inline(never)]
fn allocation_probe() -> Vec<u8> {
    std::hint::black_box(vec![42; 1024 * 1024])
}

fn allocation_cost(allocator: &impl std::alloc::GlobalAlloc) -> u128 {
    let layout = std::alloc::Layout::from_size_align(64, 8).unwrap();
    let started = std::time::Instant::now();

    for _ in 0..50000 {
        // SAFETY: Each non-null block is freed through the same allocator/layout.
        unsafe {
            let pointer = allocator.alloc(layout);

            assert!(!pointer.is_null());
            std::hint::black_box(pointer);
            allocator.dealloc(pointer, layout);
        }
    }

    started.elapsed().as_micros()
}

#[test]
fn public_allocator_reports_real_allocations_and_releases() {
    let router = Router::<NoRoute>::new(RouterConfig::default());
    let client = router.client();

    let request = |message| {
        client
            .request(Request {
                protocol: PROTOCOL_VERSION,
                runtime_id: router.runtime_id(),
                key: None,
                message,
            })
            .unwrap()
    };

    assert!(client.info().queries.iter().any(|query| query == "memory"));
    request(Message::Command(Command::RecordMemory {
        owner: "consumer".into(),
        enabled: true,
        interval: 32768,
        lease_ms: 5000,
    }));
    let value = allocation_probe();

    let Reply::Memory(before) = request(Message::Query(Query::Memory {
        owner: "consumer".into(),
        include_freed: false,
    })) else {
        panic!("memory reply");
    };

    let block = before
        .samples
        .iter()
        .find(|sample| sample.size == value.len() as u64)
        .expect("actual probe allocation captured");

    assert!(
        block.stack.iter().any(|index| before.sites[*index as usize]
            .function
            .contains("allocation_probe")),
        "native stack resolves the Rust allocation caller"
    );
    let ordinal = block.ordinal;

    // Symbolization can consume the finite lease; renew only a still-active capture.
    assert!(before.recording);
    request(Message::Command(Command::RecordMemory {
        owner: "consumer".into(),
        enabled: true,
        interval: 32768,
        lease_ms: 5000,
    }));
    drop(value);

    let Reply::Memory(after) = request(Message::Query(Query::Memory {
        owner: "consumer".into(),
        include_freed: true,
    })) else {
        panic!("memory reply");
    };

    assert!(
        after
            .samples
            .iter()
            .any(|sample| sample.ordinal == ordinal && !sample.live)
    );
    request(Message::Command(Command::RecordMemory {
        owner: "consumer".into(),
        enabled: false,
        interval: 32768,
        lease_ms: 0,
    }));
    let baseline_us = allocation_cost(&std::alloc::System);
    let dormant_us = allocation_cost(&ALLOCATOR);

    request(Message::Command(Command::RecordMemory {
        owner: "cost".into(),
        enabled: true,
        interval: 32768,
        lease_ms: 5000,
    }));
    let active_us = allocation_cost(&ALLOCATOR);

    request(Message::Command(Command::RecordMemory {
        owner: "cost".into(),
        enabled: false,
        interval: 32768,
        lease_ms: 0,
    }));
    println!(
        "50,000 64-byte alloc/free pairs (µs): System={baseline_us}, feature enabled/dormant={dormant_us}, recording={active_us}"
    );
}
