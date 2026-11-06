//! Windows Shell and native menu presentation options.
/// Native menu column boundary before an item.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Column {
    /// Continue the current column.
    #[default]
    Continue,
    /// Start another column.
    Break,
    /// Start another column with a dividing line.
    Divider,
}
/// Native menu item layout and default selection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MenuAppearance {
    /// Emphasize as the default item; at most one per menu level.
    pub default: bool,
    /// Column break behavior.
    pub column: Column,
    /// Right-to-left submenu cascade order.
    pub right_order: bool,
}

/// Native legacy balloon icon; independent from modern OS notifications.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum BalloonIcon {
    /// No icon.
    #[default]
    None,
    /// Information.
    Information,
    /// Warning.
    Warning,
    /// Error.
    Error,
    /// Owned custom icon.
    Custom(crate::Image),
}
/// Optional Windows legacy notification-area balloon.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LegacyBalloon {
    /// Plain body, at most 255 UTF-16 units; empty withdraws the current balloon.
    pub body: String,
    /// Plain title, at most 63 UTF-16 units.
    pub title: String,
    /// Native icon style.
    pub icon: BalloonIcon,
    /// Discard if it cannot be presented immediately.
    pub realtime: bool,
    /// Request silence.
    pub silent: bool,
    /// Request large icon rendering.
    pub large_icon: bool,
    /// Respect system quiet-time policy (recommended).
    pub respect_quiet_time: bool,
}
impl LegacyBalloon {
    /// Describes a balloon respecting system quiet time by default.
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            respect_quiet_time: true,
            ..Default::default()
        }
    }
    /// Validates fixed native UTF-16 buffers before any native call.
    /// # Errors
    /// Rejects embedded NULs and oversized strings.
    pub fn validate(&self) -> Result<(), crate::Error> {
        crate::text(&self.title, "balloon title", 4096)?;
        crate::text(&self.body, "balloon body", 4096)?;
        if self.title.encode_utf16().count() > 63 || self.body.encode_utf16().count() > 255 {
            return Err(crate::Error::InvalidInput(
                "legacy balloon exceeds native UTF-16 limits".into(),
            ));
        }
        Ok(())
    }
}

/// Notification-area tooltip policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TooltipMode {
    /// Use the standard native text tooltip.
    #[default]
    Standard,
    /// Emit Shell rich-tooltip open/close requests instead.
    Events,
}
/// Windows identity and Shell presentation options.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Presentation {
    /// Explicit UUID bytes in network order; otherwise derived from app/item identity.
    /// Fixed for one registration; changing requires removal/recreation.
    pub guid: Option<[u8; 16]>,
    /// Native tooltip policy.
    pub tooltip: TooltipMode,
}
