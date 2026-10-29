//! Bounded frame recording and inclusive inspected-boundary CPU wall timings.

use super::SourceLocation;
use crate::router::WindowId;
use serde::{Deserialize, Serialize};

/// An observed GPUI element callback, not a GPU or CPU sampling profiler.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FramePhase {
    /// Request-layout callback, including nested component rendering where applicable.
    Layout,
    /// Prepaint callback, including descendant work.
    Prepaint,
    /// Paint callback, including descendant work.
    Paint,
}

/// Inclusive callback duration; nested node durations must not be added together.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeTiming {
    /// Presentation identity when already known at request-layout.
    pub stable_id: Option<String>,
    /// Bounded semantic boundary label, without text content.
    pub name: String,
    /// Rust construction site for attribution.
    pub source: SourceLocation,
    /// Callback that was measured.
    pub phase: FramePhase,
    /// Start on the recording's monotonic clock, in microseconds.
    pub elapsed_us: u64,
    /// Inclusive CPU wall time in microseconds, not CPU samples.
    pub duration_us: u64,
}

/// One actual GPUI window draw and its slowest observed boundary callbacks.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FrameTiming {
    /// Monotonic runtime recording sequence.
    pub sequence: u64,
    /// Application host that owns the GPUI window.
    pub window: WindowId,
    /// Draw start on the recording clock.
    pub elapsed_us: u64,
    /// Complete `Window::draw` duration from GPUI's frame profiler.
    pub draw_us: u64,
    /// Dirty-to-draw-end latency, when GPUI observed the invalidation.
    pub dirty_to_draw_us: Option<u64>,
    /// Invalidations coalesced into this draw.
    pub invalidations: u64,
    /// Slowest callbacks, bounded to 64 entries. Durations overlap for nested nodes.
    /// Empty when callback attribution is unavailable (cached/unobserved/evicted metadata).
    pub nodes: Vec<NodeTiming>,
    /// Known callback records omitted by the per-frame limit; unobserved work is not estimated.
    pub omitted_nodes: usize,
}

/// One bounded page of retained frame recordings.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Frames {
    /// Whether the renewable recording lease is currently active.
    pub recording: bool,
    /// Chronological retained frames after the supplied cursor.
    pub frames: Vec<FrameTiming>,
    /// Continue after this sequence; zero if no frames have been returned.
    pub cursor: u64,
    /// More retained frames exist after this page.
    pub more: bool,
    /// Frames evicted from the runtime's bounded retention.
    pub dropped: u64,
    /// The supplied nonzero cursor precedes retained history.
    pub gap: bool,
}
