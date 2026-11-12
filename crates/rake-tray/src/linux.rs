//! StatusNotifierItem presentation, interpreted by the desktop host.
use crate::Image;

/// SNI category used by panel grouping policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Category {
    /// Application status, the default.
    #[default]
    ApplicationStatus,
    /// Communications.
    Communications,
    /// System services.
    SystemServices,
    /// Hardware.
    Hardware,
}

/// SNI activity; Passive does not guarantee disappearance on every panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    /// Normally active.
    #[default]
    Active,
    /// Background/passive.
    Passive,
    /// Requests attention.
    NeedsAttention,
}

/// Native rich tooltip with plain title/body and an optional image.
#[derive(Clone, Debug, Default)]
pub struct Tooltip {
    /// Plain title.
    pub title: String,
    /// Plain body; escaped before host markup interpretation.
    pub body: String,
    /// Native named or raster tooltip image.
    pub image: Option<Image>,
}

/// SNI fields whose presentation remains host-controlled.
#[derive(Clone, Debug, Default)]
pub struct Presentation {
    /// Accessible/display title, independent of tooltip.
    pub title: String,
    /// Panel grouping category.
    pub category: Category,
    /// Native activity state while requested visible.
    pub status: Status,
    /// Additional named-icon search path.
    pub icon_theme_path: String,
    /// Overlay image.
    pub overlay: Option<Image>,
    /// Attention image.
    pub attention: Option<Image>,
    /// Host-supported attention animation name.
    pub attention_movie: String,
    /// Rich tooltip, replacing the plain tooltip projection.
    pub tooltip: Option<Tooltip>,
    /// Disable the optional well-known bus name for sandbox compatibility.
    /// This registration-time option requires removing/recreating to change.
    pub sandboxed: bool,
}
