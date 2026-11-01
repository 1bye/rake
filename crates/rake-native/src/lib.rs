//! Optional native components for GPUI applications.
//!
//! Enable Rake's `native-components` feature to use this as `rake::native`.
//! Only `macos` is implemented; other targets compile without Apple dependencies.
//! Retain controls in a GPUI entity and create a fresh `.element()` each render.
//!
//! Native views are composited above GPUI. Reserve non-overlapping layout slots:
//! GPUI overlays, transforms, rounded masks and ancestor opacity cannot affect them.
//! See the crate README for composition and keyboard-focus boundaries.

/// AppKit controls and view hosting. All native objects stay on the main thread.
#[cfg(target_os = "macos")]
pub mod macos;
