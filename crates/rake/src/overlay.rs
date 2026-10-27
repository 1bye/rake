//! Experimental window-scoped overlays for GPUI and `view!` components.

#![doc = include_str!("../../../docs/experimental-overlays.md")]

mod elements;
mod state;

pub use elements::{Dialog, OverlayAnchor, OverlayHost, Popover};
pub use state::{AnchorState, OverlayHostState, OverlayState};
