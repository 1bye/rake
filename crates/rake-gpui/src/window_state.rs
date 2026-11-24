//! Native window placement and GPUI observation.
pub use rake_core::window_state::WindowState;

#[cfg(not(target_family = "wasm"))]
use rake_core::window_state::placement;

#[cfg(not(target_family = "wasm"))]
pub(crate) mod native;
