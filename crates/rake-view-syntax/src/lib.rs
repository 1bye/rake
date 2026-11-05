//! Internal view parsing and lowering for matching Rake tool versions.
//!
//! This crate is an implementation detail of `rake-macros` and `rake-app-cli`.
//! Its parse tree and lowering helpers have no stability guarantees outside
//! matching versions of those consumers. Publish it before `rake-macros`.
//!
//! Source tools must enable `source-locations` to preserve unquoted text spacing
//! outside a procedural macro. Macro hosts use stable `proc_macro` span APIs.
//!
//! Parsing does not require GPUI or build the caller's project. Lowering is kept
//! with the tree so procedural macros and source tools share the same grammar.

#[cfg(not(any(feature = "source-locations", test)))]
extern crate proc_macro;

mod view;

pub use view::preset::expand as expand_preset;
pub use view::theme::derive as derive_theme;
pub use view::{Attribute, AttributeValue, Element, MatchArm, Node, View};
