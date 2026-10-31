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

/// Development input: legacy preview gestures and literal native CDP events.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Input {
    /// Native pointer phases; a pressed button on Move supports dragging.
    Pointer {
        /// Down, up or move, without synthetic click retries.
        phase: PointerPhase,
        /// Left=1, right=2, middle=4; zero means no button.
        button: u8,
        /// Click count, from one to three.
        click_count: u8,
        /// CDP modifier bits: Alt=1, Control=2, Meta=4, Shift=8.
        modifiers: u8,
    },
    /// Dispatch a native key event to the currently focused control.
    Key {
        /// True for a press, false for a release.
        down: bool,
        /// GPUI key name, at most 128 UTF-8 bytes.
        key: String,
        /// Optional literal text associated with a press, at most 16 KiB.
        text: Option<String>,
        /// CDP modifier bits.
        modifiers: u8,
    },
    /// Insert literal text through GPUI's focused input handler, not inspection overrides.
    Text {
        /// At most 16 KiB of UTF-8 text.
        text: String,
    },
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

/// Native pointer phase, including moves with a pressed button for drag gestures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerPhase {
    /// Mouse button down.
    Down,
    /// Mouse button up.
    Up,
    /// Pointer movement.
    Move,
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
