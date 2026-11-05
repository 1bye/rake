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
