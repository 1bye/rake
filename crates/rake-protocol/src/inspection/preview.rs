//! Development preview coordinates and input, independent of native capture APIs.

use super::Layout;
use crate::router::WindowId;
use serde::{Deserialize, Serialize};

/// One inspected macOS window, in logical points relative to its outer frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Window {
    /// Runtime-scoped host identity.
    pub window: WindowId,
    /// WindowServer identity; never accepted without the matching runtime host.
    pub native_id: u32,
    /// Latest inspected presentation, sampled independently of the video stream.
    pub generation: String,
    /// Outer native window dimensions, excluding its shadow.
    pub frame: Layout,
    /// GPUI content rectangle within the captured native window.
    pub content: Layout,
    /// Whether WindowServer considers the window fully covered on this desktop.
    #[serde(default)]
    pub occluded: bool,
    /// Whether the native window is nonopaque; this does not prove that blur is enabled.
    pub translucent: bool,
}

/// An exact inspected window and a position in GPUI content logical points.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    /// Runtime-scoped host identity.
    pub window: WindowId,
    /// Native window identity from the same preview description.
    pub native_id: u32,
    /// Inspected presentation for source selection or legacy input.
    pub generation: String,
    /// Viewport dimensions/offsets in 1/64 points for live input across tree updates.
    /// Selection always requires the exact generation; omitted values retain that behavior.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry: Option<[i32; 6]>,
    /// Horizontal content coordinate.
    pub x: i32,
    /// Vertical content coordinate.
    pub y: i32,
}

/// The prototype forwards atomic primary clicks and pixel wheel deltas, not keyboard or drag input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Input {
    /// Update GPUI hover without moving the desktop cursor.
    Move,
    /// Dispatch movement, primary down and primary up in one UI-loop update.
    Click,
    /// Fractional trackpad deltas, in 1/64 logical point, with gesture lifetime.
    Wheel {
        /// Horizontal browser delta.
        dx: i32,
        /// Vertical browser delta.
        dy: i32,
        /// Gesture phase without browser-side synthetic momentum.
        phase: WheelPhase,
    },
    /// Browser wheel deltas in logical pixels, positive right/down.
    Scroll {
        /// Horizontal browser wheel delta.
        dx: i32,
        /// Vertical browser wheel delta.
        dy: i32,
    },
}

/// Browser scroll gesture lifetime, forwarded to GPUI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WheelPhase {
    /// First wheel event after an idle interval.
    Started,
    /// Continuing physical or browser-provided momentum.
    Moved,
    /// Idle timeout ends this gesture.
    Ended,
}
