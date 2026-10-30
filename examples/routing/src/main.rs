//! Native entry for the shared routing application.

#[cfg(all(feature = "memory-test", unix))]
#[global_allocator]
static ALLOCATOR: rake::memory::TrackingAllocator<std::alloc::System> =
    rake::memory::TrackingAllocator::new(std::alloc::System);

fn main() {
    rake_routing::main();
}
