//! AppKit-only appearance descriptions; rejected on other backends.
use crate::Image;

/// Width in logical AppKit points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ItemLength {
    /// Let title and image determine the width.
    #[default]
    Content,
    /// Match the status bar thickness.
    Square,
    /// Explicit positive finite width.
    Points(f64),
}
/// Native status button appearance, independent of interaction policy.
#[derive(Clone, Debug, Default)]
pub struct Appearance {
    /// Native item width.
    pub length: ItemLength,
    /// Text beside the icon; permits a text-only item.
    pub title: String,
    /// Pressed-state text.
    pub alternate_title: String,
    /// Pressed-state image.
    pub alternate_image: Option<Image>,
    /// Appearance-only disabled styling; events still work.
    pub appears_disabled: bool,
    /// Explicitly disable the native button and its interaction.
    pub disabled: bool,
    /// VoiceOver name, independent of tooltip.
    pub accessibility_label: String,
    /// VoiceOver help.
    pub accessibility_help: String,
    /// Native user removal behavior.
    pub removal: Removal,
}
/// Native menu item details.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MenuAppearance {
    /// Native secondary text (macOS 14.4+).
    pub subtitle: Option<String>,
    /// Native tooltip.
    pub tooltip: Option<String>,
    /// Option-key alternate item.
    pub alternate: bool,
    /// Indent level, from zero through fifteen.
    pub indentation: u8,
    /// Native badge (macOS 14+).
    pub badge: Option<Badge>,
    /// Custom off/on/mixed state images.
    pub state_images: Option<StateImages>,
}

/// Native menu badge content and semantic style.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Badge {
    /// Plain count.
    Count(u32),
    /// Plain text.
    Text(String),
    /// New items count.
    NewItems(u32),
    /// Updates count.
    Updates(u32),
    /// Alerts count.
    Alerts(u32),
}
/// Explicit native checkmark images.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StateImages {
    /// Unchecked image.
    pub off: Option<Image>,
    /// Checked image.
    pub on: Option<Image>,
    /// Indeterminate image.
    pub mixed: Option<Image>,
}
/// Native palette selection behavior (macOS 14+).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Selection {
    /// Native automatic policy.
    #[default]
    Automatic,
    /// Allow selecting one item.
    One,
    /// Allow selecting multiple items.
    Any,
}
/// Native menu-wide layout, separate from menu item state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuLayout {
    /// Minimum width in logical points.
    pub minimum_width: u16,
    /// Show the state/checkmark column.
    pub shows_state_column: bool,
    /// Palette presentation (macOS 14+).
    pub palette: bool,
    /// Native selection policy (macOS 14+ when not Automatic).
    pub selection: Selection,
    /// Enable native context-menu plug-ins explicitly.
    pub context_plugins: bool,
}
impl Default for MenuLayout {
    fn default() -> Self {
        Self {
            minimum_width: 0,
            shows_state_column: true,
            palette: false,
            selection: Selection::Automatic,
            context_plugins: false,
        }
    }
}

/// User removal never bypasses Rake's guarded application quit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Removal {
    /// Native user removal is disabled.
    #[default]
    Prevent,
    /// Permit removal; observe visibility and restore with Handle::show.
    Allow,
    /// Request guarded quit when the user removes the item.
    RequestQuit,
}
/// Current status-bar dimensions, in AppKit logical points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    /// True if the native bar is vertical.
    pub vertical: bool,
    /// Native status-bar thickness.
    pub thickness: f64,
    /// Current item length.
    pub length: f64,
    /// Actual native visible property, independent of requested visibility.
    pub visible: bool,
}
