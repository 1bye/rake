//! Display-aware placement evidence for a separate application popup window.
use crate::Rect;

/// Native icon and work area translated to GPUI's target-display logical units.
///
/// AppKit uses display-local top-left coordinates. Windows uses screen coordinates
/// divided by the target monitor's effective DPI, matching GPUI's native adapter.
#[derive(Clone, Debug, PartialEq)]
pub struct PopupAnchor {
    /// Native display identifier understood by the selected GPUI platform adapter.
    pub display_id: u64,
    /// Current icon rectangle.
    pub icon: Rect,
    /// Display work area excluding dock/taskbar/menu bar.
    pub work_area: Rect,
}
