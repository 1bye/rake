//! Experimental window-scoped overlays for GPUI and `view!` components.

mod elements;
mod state;

pub use elements::{Dialog, OverlayAnchor, OverlayHost, Popover};
pub use state::{AnchorState, OverlayHostState, OverlayState};
