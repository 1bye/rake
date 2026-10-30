//! Opt-in Unix allocator sampling. Disabled builds contain no allocator hooks.
//!
//! Install the wrapper in the executable, around its existing global allocator:
//! ```
//! #[global_allocator]
//! static ALLOCATOR: rake::memory::TrackingAllocator<std::alloc::System> =
//!     rake::memory::TrackingAllocator::new(std::alloc::System);
//! ```
//! Only leased recordings unwind stacks. Hooks never inspect application memory.
//! The bounded recorder uses nonblocking admission; capture/symbolization suppress
//! this thread's instrumentation. C/C++ allocations, allocator overhead, GPU memory
//! and allocations optimized away by Rust are outside coverage.

use rake_protocol::inspection::memory::{
    MAX_FREED_SAMPLES, MAX_LIVE_SAMPLES, MAX_SITES, STACK_DEPTH, Sample, Site, Snapshot,
};
use std::{
    alloc::{GlobalAlloc, Layout},
    cell::Cell,
    collections::{BTreeMap, HashMap, VecDeque},
    sync::{
        Mutex, OnceLock,
        atomic::{
            AtomicBool, AtomicU8, AtomicU64, AtomicUsize,
            Ordering::{AcqRel, Acquire, Relaxed, Release},
        },
    },
    time::Instant,
};

static INSTALLED: AtomicBool = AtomicBool::new(false);
static UNTIL: AtomicU64 = AtomicU64::new(0);
static OMITTED: AtomicU64 = AtomicU64::new(0);
static FREED_OMITTED: AtomicU64 = AtomicU64::new(0);
static NEXT: AtomicU64 = AtomicU64::new(1);
static SEED: AtomicU64 = AtomicU64::new(0x9e3779b97f4a7c15);
static EPOCH: OnceLock<Instant> = OnceLock::new();
static STATE: OnceLock<Mutex<State>> = OnceLock::new();
// A contended free is deferred without blocking or allocating in the hook.
const FREE_SLOTS: usize = 256;
static PENDING_FREES: AtomicUsize = AtomicUsize::new(0);
static LOST_FREE_BEFORE: AtomicU64 = AtomicU64::new(0);
static FREES: [Free; FREE_SLOTS] = [const {
    Free {
        state: AtomicU8::new(0),
        pointer: AtomicUsize::new(0),
        before: AtomicU64::new(0),
    }
}; FREE_SLOTS];

struct Free {
    state: AtomicU8,
    pointer: AtomicUsize,
    before: AtomicU64,
}
thread_local! {
    static BUSY: Cell<bool> = const { Cell::new(false) };
    static RANDOM: Cell<u64> = const { Cell::new(0) };
}

/// Delegates allocation to `A`, adding bounded sampling only in profiling builds.
///
/// This wrapper does not register a global allocator itself. Applications retain
/// control over their allocator and must gate the static with their dev feature.
#[derive(Debug)]
pub struct TrackingAllocator<A>(A);

impl<A> TrackingAllocator<A> {
    /// Wraps an allocator without allocating or starting a recording.
    pub const fn new(allocator: A) -> Self {
        Self(allocator)
    }
}

struct Suppress;
impl Suppress {
    fn enter() -> Option<Self> {
        BUSY.try_with(|busy| if busy.replace(true) { None } else { Some(Self) })
            .ok()
            .flatten()
    }
}
impl Drop for Suppress {
    fn drop(&mut self) {
        let _ = BUSY.try_with(|busy| busy.set(false));
    }
}

#[derive(Clone)]
struct Raw {
    ordinal: u64,
    size: u64,
    estimate: u64,
    stack: [usize; STACK_DEPTH],
    depth: usize,
    live: bool,
}
struct State {
    owner: String,
    generation: u64,
    interval: u64,
    live: HashMap<usize, Raw>,
    freed: VecDeque<Raw>,
    symbols: BTreeMap<usize, Site>,
    lost_free_before: u64,
}

fn queue_free(pointer: *mut u8, before: u64) {
    let start = before as usize % FREE_SLOTS;
    for offset in 0..FREE_SLOTS {
        let slot = &FREES[(start + offset) % FREE_SLOTS];
        if slot.state.compare_exchange(0, 1, AcqRel, Relaxed).is_ok() {
            slot.pointer.store(pointer as usize, Relaxed);
            slot.before.store(before, Relaxed);
            PENDING_FREES.fetch_add(1, Release);
            slot.state.store(2, Release);
            return;
        }
    }
    // If even the fixed queue is full, discard affected observations rather than
    // presenting potentially freed blocks as live. Application blocks are untouched.
    LOST_FREE_BEFORE.fetch_max(before, Relaxed);
    OMITTED.fetch_add(1, Relaxed);
}

fn retain_freed(state: &mut State, mut sample: Raw) {
    sample.live = false;
    if state.freed.len() == MAX_FREED_SAMPLES {
        state.freed.pop_front();
        FREED_OMITTED.fetch_add(1, Relaxed);
    }
    state.freed.push_back(sample);
}

fn drain_frees(state: &mut State) {
    if PENDING_FREES.load(Acquire) != 0 {
        for slot in &FREES {
            if slot.state.compare_exchange(2, 1, AcqRel, Relaxed).is_err() {
                continue;
            }
            let pointer = slot.pointer.load(Relaxed);
            let before = slot.before.load(Relaxed);
            slot.state.store(0, Release);
            PENDING_FREES.fetch_sub(1, AcqRel);
            // The ticket predates the underlying free/realloc. A reused address
            // has a newer ordinal and must never be removed by an older free.
            if state
                .live
                .get(&pointer)
                .is_some_and(|sample| sample.ordinal < before)
                && let Some(sample) = state.live.remove(&pointer)
            {
                retain_freed(state, sample);
            }
        }
    }
    let lost = LOST_FREE_BEFORE.load(Relaxed);
    if lost > state.lost_free_before {
        state.live.retain(|_, sample| {
            if sample.ordinal <= lost {
                OMITTED.fetch_add(1, Relaxed);
                false
            } else {
                true
            }
        });
        state.lost_free_before = lost;
    }
}

fn now() -> u64 {
    EPOCH
        .get()
        .map_or(0, |epoch| epoch.elapsed().as_millis() as u64 + 1)
}
fn active() -> bool {
    let until = UNTIL.load(Relaxed);
    until != 0 && now() < until
}
pub(crate) fn installed() -> bool {
    INSTALLED.load(Relaxed)
}

pub(crate) fn record(
    owner: &str,
    enabled: bool,
    interval: u64,
    lease_ms: u64,
) -> Result<(), String> {
    if !installed() {
        return Err("native memory allocator wrapper is not installed".into());
    }
    if owner.is_empty() || owner.len() > 128 {
        return Err("invalid memory recorder owner".into());
    }
    if enabled && (!(256..=16 * 1024 * 1024).contains(&interval) || !(1..=5000).contains(&lease_ms))
    {
        return Err("memory interval must be 256–16777216 bytes and lease 1–5000 ms".into());
    }
    let _suppress = Suppress::enter();
    EPOCH.get_or_init(Instant::now);
    let state = STATE.get_or_init(|| {
        Mutex::new(State {
            owner: String::new(),
            generation: 0,
            interval,
            live: HashMap::with_capacity(MAX_LIVE_SAMPLES),
            freed: VecDeque::with_capacity(MAX_FREED_SAMPLES),
            symbols: BTreeMap::new(),
            lost_free_before: 0,
        })
    });
    let mut state = state.lock().map_err(|_| "memory recorder lock poisoned")?;
    drain_frees(&mut state);
    if active() && state.owner != owner {
        return Err("another client owns the memory recorder".into());
    }
    if !enabled {
        if state.owner != owner {
            return Err("memory recorder owner expired".into());
        }
        UNTIL.store(0, Relaxed);
        return Ok(());
    }
    if active() && state.interval != interval {
        return Err("cannot change interval during a recording".into());
    }
    if !active() && state.owner == owner {
        return Err("memory recording lease expired; start with a new owner".into());
    }
    if !active() {
        state.owner = owner.into();
        state.generation += 1;
        state.interval = interval;
        state.live.clear();
        state.freed.clear();
        state.symbols.clear();
        OMITTED.store(0, Relaxed);
        FREED_OMITTED.store(0, Relaxed);
    }
    UNTIL.store(now().saturating_add(lease_ms), Relaxed);
    Ok(())
}

fn random() -> f64 {
    RANDOM.with(|cell| {
        let mut value = cell.get();
        if value == 0 {
            value = SEED.fetch_add(0x9e3779b97f4a7c15, Relaxed) | 1;
        }
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        cell.set(value);
        ((value >> 11) as f64 + 0.5) / ((1u64 << 53) as f64)
    })
}

fn allocation(pointer: *mut u8, size: usize) {
    if pointer.is_null() || !active() {
        return;
    }
    let Some(_suppress) = Suppress::enter() else {
        return;
    };
    let Some(state) = STATE.get() else {
        return;
    };
    let (generation, interval) = match state.try_lock() {
        Ok(mut state) => {
            drain_frees(&mut state);
            (state.generation, state.interval)
        }
        Err(_) => {
            OMITTED.fetch_add(1, Relaxed);
            return;
        }
    };
    let probability = -(-(size as f64) / interval as f64).exp_m1();
    if random() >= probability {
        return;
    }
    let mut sample = Raw {
        ordinal: NEXT.fetch_add(1, Relaxed),
        size: size as u64,
        estimate: ((size as f64 / probability).round() as u64).max(size as u64),
        stack: [0; STACK_DEPTH],
        depth: 0,
        live: true,
    };
    // SAFETY: On Unix this pinned backtrace version uses _Unwind_Backtrace with
    // per-invocation state. No global symbol resolver is accessed. The callback
    // only copies PCs, never allocates or panics; TLS prevents allocator recursion.
    unsafe {
        backtrace::trace_unsynchronized(|frame| {
            if sample.depth == STACK_DEPTH {
                return false;
            }
            sample.stack[sample.depth] = (frame.ip() as usize).saturating_sub(1);
            sample.depth += 1;
            true
        });
    }
    if let Ok(mut state) = state.try_lock() {
        drain_frees(&mut state);
        if state.generation == generation && active() {
            if state.live.len() < MAX_LIVE_SAMPLES {
                state.live.insert(pointer as usize, sample);
            } else {
                OMITTED.fetch_add(1, Relaxed);
            }
        }
    } else {
        OMITTED.fetch_add(1, Relaxed);
    }
}

fn remove(pointer: *mut u8) -> Result<Option<(u64, Raw)>, u64> {
    if !active() {
        return Ok(None);
    }
    let Some(_suppress) = Suppress::enter() else {
        return Ok(None);
    };
    let Some(shared) = STATE.get() else {
        return Ok(None);
    };
    let mut state = match shared.try_lock() {
        Ok(state) => state,
        Err(_) => {
            return Err(NEXT.fetch_add(1, Relaxed));
        }
    };
    drain_frees(&mut state);
    let generation = state.generation;
    Ok(state
        .live
        .remove(&(pointer as usize))
        .map(|sample| (generation, sample)))
}

fn deallocation(pointer: *mut u8) {
    if !active() {
        return;
    }
    let Some(_suppress) = Suppress::enter() else {
        return;
    };
    let Some(shared) = STATE.get() else {
        return;
    };
    if let Ok(mut state) = shared.try_lock() {
        drain_frees(&mut state);
        if let Some(sample) = state.live.remove(&(pointer as usize)) {
            retain_freed(&mut state, sample);
        }
    } else {
        queue_free(pointer, NEXT.fetch_add(1, Relaxed));
    }
}
fn freed(previous: Option<(u64, Raw)>, pointer: *mut u8, failed: bool) {
    let Some((generation, sample)) = previous else {
        return;
    };
    let Some(_suppress) = Suppress::enter() else {
        return;
    };
    if let Some(state) = STATE.get() {
        if let Ok(mut state) = state.try_lock() {
            if generation != state.generation {
                return;
            }
            if failed {
                if state.live.len() < MAX_LIVE_SAMPLES {
                    state.live.insert(pointer as usize, sample);
                } else {
                    OMITTED.fetch_add(1, Relaxed);
                }
            } else {
                retain_freed(&mut state, sample);
            }
        } else if failed {
            OMITTED.fetch_add(1, Relaxed);
        } else {
            FREED_OMITTED.fetch_add(1, Relaxed);
        }
    }
}

// SAFETY: Every operation delegates the exact caller pointer/layout to A. No hook
// dereferences application blocks. Failed realloc preserves the original sample.
unsafe impl<A: GlobalAlloc> GlobalAlloc for TrackingAllocator<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        INSTALLED.store(true, Relaxed);
        // SAFETY: The caller supplies A's valid allocation layout.
        let pointer = unsafe { self.0.alloc(layout) };
        allocation(pointer, layout.size());
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        INSTALLED.store(true, Relaxed);
        // SAFETY: Delegating retains A's zero-initialization and layout contract.
        let pointer = unsafe { self.0.alloc_zeroed(layout) };
        allocation(pointer, layout.size());
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        deallocation(pointer);
        // SAFETY: The caller guarantees pointer and layout belong to A.
        unsafe {
            self.0.dealloc(pointer, layout);
        }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let previous = remove(pointer);
        // SAFETY: The caller provides a live A block and a nonzero new size.
        let next = unsafe { self.0.realloc(pointer, layout, size) };
        match previous {
            Ok(previous) => freed(previous, pointer, next.is_null()),
            Err(before) if !next.is_null() => queue_free(pointer, before),
            Err(_) => {} // Failed realloc retains its original live allocation.
        }
        allocation(next, size);
        next
    }
}

pub(crate) fn snapshot(owner: &str, include_freed: bool) -> Result<Snapshot, String> {
    let _suppress = Suppress::enter();
    let shared = STATE.get().ok_or("memory recorder has not started")?;
    let mut state = shared.lock().map_err(|_| "memory recorder lock poisoned")?;
    if state.owner != owner {
        return Err("memory recorder owner expired".into());
    }
    drain_frees(&mut state);
    let mut raw: Vec<_> = state.live.values().cloned().collect();
    if include_freed {
        raw.extend(state.freed.iter().cloned());
    }
    raw.sort_by_key(|sample| sample.ordinal);
    let generation = state.generation;
    let interval = state.interval;
    let recording = active();
    let mut symbols = std::mem::take(&mut state.symbols);
    drop(state);
    // Symbolization may allocate and read debug files. It never runs in a hook
    // or while holding the allocator table lock.
    let mut priority: Vec<_> = raw.iter().collect();
    priority.sort_by_key(|sample| {
        (
            std::cmp::Reverse(sample.estimate),
            std::cmp::Reverse(sample.live),
            sample.ordinal,
        )
    });
    let mut selected = std::collections::BTreeSet::new();
    for sample in priority {
        for &address in &sample.stack[..sample.depth] {
            if selected.len() < MAX_SITES - 1 {
                selected.insert(address);
            }
        }
    }
    symbols.retain(|address, _| selected.contains(address));
    for address in selected {
        if symbols.contains_key(&address) {
            continue;
        }
        let mut site = Site {
            address: format!("0x{address:x}"),
            function: format!("native 0x{address:x}"),
            file: None,
            line: None,
            column: None,
        };
        backtrace::resolve(address as *mut _, |symbol| {
            if let Some(name) = symbol.name() {
                site.function = bounded(format!("{name:#}"), 256);
            }
            site.file = symbol
                .filename()
                .and_then(|file| file.to_str())
                .filter(|file| file.len() <= 512)
                .map(str::to_owned);
            site.line = symbol.lineno();
            site.column = symbol.colno();
        });
        symbols.insert(address, site);
    }
    let mut sites = vec![Site {
        address: "unresolved".into(),
        function: "(native frames omitted)".into(),
        file: None,
        line: None,
        column: None,
    }];
    let mut indices = BTreeMap::new();
    for (&address, site) in &symbols {
        indices.insert(address, sites.len() as u16);
        sites.push(site.clone());
    }
    let freed_omitted = FREED_OMITTED.load(Relaxed);
    let mut omitted =
        OMITTED
            .load(Relaxed)
            .saturating_add(if include_freed { freed_omitted } else { 0 });
    let samples = raw
        .into_iter()
        .map(|sample| {
            let stack = sample.stack[..sample.depth]
                .iter()
                .filter_map(|address| {
                    let index = indices.get(address).copied().unwrap_or_else(|| {
                        omitted += 1;
                        0
                    });
                    let name = &sites[index as usize].function;
                    (!name.contains("rake::memory::")
                        && !name.contains("backtrace::")
                        && !name.contains("__rust_alloc")
                        && !name.contains("__rust_realloc")
                        && !name.starts_with("alloc::alloc::"))
                    .then_some(index)
                })
                .collect();
            Sample {
                ordinal: sample.ordinal,
                size: sample.size,
                estimated_bytes: sample.estimate,
                live: sample.live,
                stack,
            }
        })
        .collect();
    let mut state = shared.lock().map_err(|_| "memory recorder lock poisoned")?;
    if state.generation == generation {
        state.symbols = symbols;
    }
    Ok(Snapshot {
        generation,
        recording,
        interval,
        omitted,
        freed_omitted,
        sites,
        samples,
    })
}

fn bounded(mut text: String, limit: usize) -> String {
    let mut length = text.len().min(limit);
    while !text.is_char_boundary(length) {
        length -= 1;
    }
    text.truncate(length);
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    struct FailedRealloc;
    // SAFETY: Delegates valid layouts/pointers to System; null realloc retains the old block.
    unsafe impl GlobalAlloc for FailedRealloc {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            // SAFETY: Caller provides a valid layout for System.
            unsafe { std::alloc::System.alloc(layout) }
        }
        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            // SAFETY: Caller supplies the original System block/layout.
            unsafe { std::alloc::System.dealloc(pointer, layout) }
        }
        unsafe fn realloc(&self, _: *mut u8, _: Layout, _: usize) -> *mut u8 {
            std::ptr::null_mut()
        }
    }
    #[test]
    fn allocator_preserves_layout_contents_and_sample_lifetimes() {
        let allocator = TrackingAllocator::new(std::alloc::System);
        let layout = Layout::from_size_align(65536, 32).unwrap();
        // SAFETY: The test pairs each allocation with its exact allocator/layout.
        unsafe {
            let warm = allocator.alloc(layout);
            assert!(!warm.is_null());
            allocator.dealloc(warm, layout);
            record("test", true, 256, 5000).unwrap();
            assert!(record("other", true, 256, 5000).is_err());
            let pointer = allocator.alloc_zeroed(layout);
            assert!(!pointer.is_null());
            assert_eq!((pointer as usize) % 32, 0);
            assert_eq!(*pointer, 0);
            *pointer = 42;
            let next = allocator.realloc(pointer, layout, 131072);
            assert!(!next.is_null());
            assert_eq!(*next, 42);
            let before = snapshot("test", false).unwrap();
            assert_eq!(before.samples.len(), 1);
            assert_eq!(before.samples[0].size, 131072);
            allocator.dealloc(next, Layout::from_size_align(131072, 32).unwrap());
            assert!(snapshot("test", false).unwrap().samples.is_empty());
            assert_eq!(snapshot("test", true).unwrap().samples.len(), 2);
            record("test", false, 0, 0).unwrap();
            assert!(!snapshot("test", false).unwrap().recording);
            record("other", true, 256, 1).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(3));
            assert!(!active());
            assert!(record("other", true, 256, 5000).is_err());
            record("failure", true, 256, 5000).unwrap();
            let allocator = TrackingAllocator::new(FailedRealloc);
            let pointer = allocator.alloc(layout);
            assert!(!pointer.is_null());
            *pointer = 19;
            let ordinal = STATE.get().unwrap().lock().unwrap().live[&(pointer as usize)].ordinal;
            assert!(
                allocator
                    .realloc(pointer, layout, layout.size() * 2)
                    .is_null()
            );
            assert_eq!(*pointer, 19);
            assert_eq!(
                STATE.get().unwrap().lock().unwrap().live[&(pointer as usize)].ordinal,
                ordinal
            );
            let held = STATE.get().unwrap().lock().unwrap();
            assert!(
                allocator
                    .realloc(pointer, layout, layout.size() * 2)
                    .is_null()
            );
            drop(held);
            assert_eq!(
                STATE.get().unwrap().lock().unwrap().live[&(pointer as usize)].ordinal,
                ordinal
            );
            let held = STATE.get().unwrap().lock().unwrap();
            allocator.dealloc(pointer, layout);
            drop(held);
            assert!(
                !snapshot("failure", false)
                    .unwrap()
                    .samples
                    .iter()
                    .any(|sample| sample.ordinal == ordinal),
                "a contended free must not become a stale live sample"
            );
            record("failure", false, 0, 0).unwrap();
            record("history", true, 256, 5000).unwrap();
            let allocator = TrackingAllocator::new(std::alloc::System);
            for _ in 0..MAX_FREED_SAMPLES + 1 {
                let pointer = allocator.alloc(layout);
                assert!(!pointer.is_null());
                allocator.dealloc(pointer, layout);
            }
            let live = snapshot("history", false).unwrap();
            assert_eq!(
                live.omitted, 0,
                "evicted freed history cannot make live-only capture partial"
            );
            assert_eq!(live.freed_omitted, 1);
            let churn = snapshot("history", true).unwrap();
            assert_eq!(churn.omitted, 1);
            assert_eq!(churn.samples.len(), MAX_FREED_SAMPLES);
            record("history", false, 0, 0).unwrap();
            record("reuse", true, 256, 5000).unwrap();
            let pointer = allocator.alloc(layout);
            assert!(!pointer.is_null());
            let ordinal = STATE.get().unwrap().lock().unwrap().live[&(pointer as usize)].ordinal;
            queue_free(pointer, ordinal - 1);
            assert!(
                snapshot("reuse", false)
                    .unwrap()
                    .samples
                    .iter()
                    .any(|sample| sample.ordinal == ordinal),
                "an older free cannot delete a reused address's newer sample"
            );
            let held = STATE.get().unwrap().lock().unwrap();
            for index in 0..FREE_SLOTS {
                queue_free((0x1000 + index) as *mut u8, NEXT.fetch_add(1, Relaxed));
            }
            queue_free(pointer, NEXT.fetch_add(1, Relaxed));
            drop(held);
            let lost = snapshot("reuse", false).unwrap();
            assert!(lost.samples.is_empty());
            assert!(
                lost.omitted > 0,
                "overflow discards uncertain observations explicitly"
            );
            allocator.dealloc(pointer, layout);
            record("reuse", false, 0, 0).unwrap();
        }
    }
}
