//! Internal view parsing and lowering for matching Rake tool versions.
//!
//! This crate is an implementation detail of `rake-macros` and `rake-app-cli`.
//! Its parse tree and lowering helpers have no stability guarantees outside
//! matching versions of those consumers. Publish it before `rake-macros`.
//!
//! Parsing does not require GPUI or build the caller's project. Lowering is kept
//! with the tree so procedural macros and source tools share the same grammar.

mod view;

pub use view::preset::expand as expand_preset;
pub use view::theme::derive as derive_theme;
pub use view::{Attribute, AttributeValue, Element, Layout, LayoutKind, MatchArm, Node, View};

pub use view::classes::lint::style_classes;
