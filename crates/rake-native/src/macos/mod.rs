//! AppKit owns drawing, events and accessibility; GPUI owns layout and mounting.
//!
//! ```no_run
//! # fn main() -> Result<(), rake_native::macos::NativeError> {
//! use rake_native::macos::{Button, ButtonStyle};
//! use gpui::{prelude::*, px};
//!
//! let button = Button::new("Save", ButtonStyle::LiquidGlass)?;
//! let element = button.element().w(px(100.)).h(px(32.));
//! # Ok(())
//! # }
//! ```

mod accessibility;
mod action;
mod button;
mod segmented;
mod view;

pub use button::{Button, ButtonStyle};
pub use segmented::SegmentedControl;
pub use view::{NativeElement, NativeView};

/// Matching Objective-C runtime for constructing custom AppKit views.
pub use objc2;
/// Matching AppKit bindings accepted by [`NativeView::from_appkit`].
pub use objc2_app_kit as appkit;
/// Matching Foundation bindings for custom AppKit views.
pub use objc2_foundation as foundation;

/// Failure to construct or configure a native component.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum NativeError {
    /// AppKit objects must be created on the application's main thread.
    #[error("native components must be created on the main thread")]
    NotMainThread,
    /// The supplied SF Symbol is unavailable on this macOS version.
    #[error("SF Symbol is unavailable: {0}")]
    SymbolUnavailable(String),
    /// A segmented control requires at least one label.
    #[error("a segmented control requires at least one label")]
    EmptySegments,
    /// The requested segment does not exist.
    #[error("segment {index} is outside a control with {count} segments")]
    InvalidSegment {
        /// Requested zero-based index.
        index: usize,
        /// Number of available segments.
        count: usize,
    },
}
