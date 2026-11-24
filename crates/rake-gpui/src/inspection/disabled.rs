//! Inspection contracts without capture, editing, recording or retained state.
//!
//! Enable `rake/devtools` to compile instrumentation. Manual identity builders
//! return their input unchanged; use `inspect_with` for lazily constructed names.

pub use rake_protocol::inspection::*;

#[derive(Clone, Default)]
pub(crate) struct Store {}

#[cfg(test)]
impl rake_core::inspection::InspectionHost for Store {}

#[cfg(feature = "gpui")]
mod view {
    use gpui::IntoElement;

    /// The original element type when inspection is compiled out.
    pub type Inspected<E> = E;

    /// Returns the original value without converting or wrapping it.
    #[inline(always)]
    pub fn inspect<E: IntoElement>(element: E) -> E {
        element
    }

    /// Identity builders when inspection is compiled out. No metadata is retained.
    /// Arguments to ordinary methods still follow Rust's evaluation rules; use
    /// `inspect_with` to avoid constructing a dynamic name in production.
    pub trait InspectElement: IntoElement {
        /// Returns the original value, discarding the supplied identity.
        #[inline(always)]
        fn inspect(self, _: impl Into<String>) -> Self {
            self
        }

        /// Returns the original value without invoking the name builder.
        #[inline(always)]
        fn inspect_with(self, _: impl FnOnce() -> String) -> Self {
            self
        }

        /// Returns the original value without retaining handler metadata.
        #[inline(always)]
        fn inspect_handler(self, _: impl Into<String>) -> Self {
            self
        }

        /// Returns the original value without retaining an identity.
        #[inline(always)]
        fn semantic_id(self, _: impl Into<String>) -> Self {
            self
        }

        /// Returns the original value without cloning the focus handle.
        #[inline(always)]
        fn focus_handle(self, _: &gpui::FocusHandle) -> Self {
            self
        }

        /// Returns the original value without cloning the scroll handle.
        #[inline(always)]
        fn scroll_handle(self, _: &gpui::ScrollHandle) -> Self {
            self
        }
    }

    impl<T: IntoElement> InspectElement for T {}

    pub(crate) trait InspectRoot: IntoElement {
        #[inline(always)]
        fn root(self, _: super::Store, _: u64, _: crate::router::WindowId) -> Self {
            self
        }
    }

    impl<T: IntoElement> InspectRoot for T {}
}

#[cfg(feature = "gpui")]
pub(crate) use view::InspectRoot;
#[cfg(feature = "gpui")]
pub use view::{InspectElement, Inspected, inspect};

#[cfg(test)]
mod tests {
    #[test]
    fn disabled_store_has_no_retained_state() {
        assert_eq!(std::mem::size_of::<super::Store>(), 0);
        #[cfg(feature = "gpui")]
        assert!(!crate::control::inspection_enabled(
            crate::control::ControlConfig::FromEnvironment
        ));
    }
}
