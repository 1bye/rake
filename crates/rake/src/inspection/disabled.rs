//! Inspection contracts without capture, editing, recording or retained state.
//!
//! Enable `rake/devtools` to compile instrumentation. Manual identity builders
//! return their input unchanged; use `inspect_with` for lazily constructed names.

pub use rake_protocol::inspection::*;

#[derive(Clone, Default)]
pub(crate) struct Store {}

impl Store {
    pub(crate) fn edit_text(&self, _: &str, _: &str, _: &TextEdit) -> Result<(), InspectionError> {
        Err(InspectionError::Unavailable)
    }
    pub(crate) fn status(&self) -> Status {
        Status::Unavailable
    }
    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot {
            status: Status::Unavailable,
            capture: "0".into(),
            router_revision: 0,
            elements: Vec::new(),
            omitted: 0,
            coverage: "Inspection was compiled out; enable rake/devtools".into(),
        }
    }
    pub(crate) fn tree_page(&self, _: Option<&str>, _: usize) -> Result<TreePage, InspectionError> {
        Err(InspectionError::Unavailable)
    }
    pub(crate) fn element(&self, _: &str) -> Result<Element, InspectionError> {
        Err(InspectionError::Unavailable)
    }
    pub(crate) fn pick(&self) -> Result<Pick, InspectionError> {
        Err(InspectionError::Unavailable)
    }
    pub(crate) fn highlight(&self, _: Option<&str>, _: &str) -> Result<(), InspectionError> {
        Err(InspectionError::Unavailable)
    }
    pub(crate) fn inspect_mode(&self, _: bool, _: u64) -> Result<(), InspectionError> {
        Err(InspectionError::Unavailable)
    }
    pub(crate) fn navigate(
        &self,
        _: &str,
        _: &str,
        _: NavigationAction,
    ) -> Result<(), InspectionError> {
        Err(InspectionError::Unavailable)
    }
    pub(crate) fn edit_style(
        &self,
        _: &str,
        _: &str,
        _: &StyleEdit,
    ) -> Result<(), InspectionError> {
        Err(InspectionError::Unavailable)
    }
    pub(crate) fn record_frames(&self, _: bool, _: u64) -> Result<(), InspectionError> {
        Err(InspectionError::RecordingUnavailable)
    }
    pub(crate) fn frames(&self, _: u64) -> Frames {
        Frames {
            recording: false,
            frames: Vec::new(),
            cursor: 0,
            more: false,
            dropped: 0,
            gap: false,
        }
    }
}

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
        assert_eq!(super::Store::default().status(), super::Status::Unavailable);
        #[cfg(feature = "gpui")]
        assert!(!crate::control::ControlConfig::FromEnvironment.inspection_enabled());
    }
}
