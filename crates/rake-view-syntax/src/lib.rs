//! Shared parsing and lowering for Rake's experimental `view!` syntax.
//!
//! Parsing does not require GPUI or build the caller's project. Lowering is kept
//! with the tree so procedural macros and source tools share the same grammar.

mod view;

pub use view::preset::expand as expand_preset;
pub use view::theme::derive as derive_theme;
pub use view::{Attribute, AttributeValue, Element, MatchArm, Node, View};
