//! Bounded native allocation samples, not a Rust ownership graph or process RSS.

use serde::{Deserialize, Serialize};

/// Maximum live allocation samples retained by one process-wide recorder.
pub const MAX_LIVE_SAMPLES: usize = 512;
/// Maximum recently freed samples available for allocation-churn profiles.
pub const MAX_FREED_SAMPLES: usize = 512;
/// Maximum symbolized native addresses per recording.
pub const MAX_SITES: usize = 256;
/// Maximum native frames captured per sampled allocation.
pub const STACK_DEPTH: usize = 24;

/// Symbolized allocation frame; absent source information remains unknown.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Site {
    /// Native code address, stable within this process only.
    pub address: String,
    /// Demangled function, or an explicitly unresolved native address.
    pub function: String,
    /// Rust debug-info file, if available. Consumers must apply source confinement.
    pub file: Option<String>,
    /// One-based source line, if available.
    pub line: Option<u32>,
    /// One-based source column, if available.
    pub column: Option<u32>,
}

/// One sampled allocator block. No pointer or dereference capability is exported.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sample {
    /// Process-wide monotonic identity, never recycled on a new recording.
    pub ordinal: u64,
    /// Actual requested block size, excluding allocator bookkeeping.
    pub size: u64,
    /// Inverse-probability estimate of allocation bytes represented by this sample.
    pub estimated_bytes: u64,
    /// Whether the sampled block was still allocated at capture time.
    pub live: bool,
    /// Native stack in leaf-first order, indexing [`Snapshot::sites`].
    pub stack: Vec<u16>,
}

/// Owned capture of one recorder generation, bounded independently of Chrome.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    /// Recorder generation; stable identities are meaningful only within it.
    pub generation: u64,
    /// Whether the owner still has an active recording lease.
    pub recording: bool,
    /// Poisson allocation sampling interval in requested bytes.
    pub interval: u64,
    /// Known omissions from contention, recursion or bounded retention.
    pub omitted: u64,
    /// Captured symbols, at most [`MAX_SITES`].
    pub sites: Vec<Site>,
    /// Live and optionally recently freed samples, ordered by ordinal.
    pub samples: Vec<Sample>,
}
