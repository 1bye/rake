//! Framework-free inspection wire types and control dispatch provided by the host.

pub use rake_protocol::inspection::*;

#[doc(hidden)]
pub trait InspectionHost: std::any::Any {
    fn window_page(
        &self,
        _: crate::router::WindowId,
        _: Option<&str>,
        _: usize,
    ) -> Result<WindowPage, InspectionError> {
        Err(InspectionError::Unavailable)
    }

    fn preview_enabled(&self) -> bool {
        false
    }

    fn preview_windows(&self) -> Vec<preview::Window> {
        Vec::new()
    }

    fn preview_element(&self, _: &preview::Point) -> Result<Element, InspectionError> {
        Err(InspectionError::Unavailable)
    }

    fn preview_input(&self, _: &preview::Point, _: &preview::Input) -> Result<(), InspectionError> {
        Err(InspectionError::Unavailable)
    }

    fn edit_text(&self, _: &str, _: &str, _: &TextEdit) -> Result<(), InspectionError> {
        Err(InspectionError::Unavailable)
    }

    fn status(&self) -> Status {
        Status::Unavailable
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            status: Status::Unavailable,
            capture: "0".into(),
            router_revision: 0,
            locale: None,
            elements: Vec::new(),
            omitted: 0,
            coverage: if cfg!(feature = "devtools") {
                "Rake inspection boundaries, including automatic view! nodes; opaque GPUI descendants and reused cached subtrees are not enumerated. Layout is before clipping; content_mask records the visible intersection. Authored styles are separate from live resolved Div/Stateful<Div>/view! button styles, which include current interaction states and inherited text. Relative layout lengths retain their units. Box geometry resolves absolute edges from the layout-phase style; used auto/percentage edges and scrollbar metrics may be unavailable. Style origins describe observed defaults, inherited text, authored base, interaction changes and measured size; macro-authored style attribute locations are captured separately, while opaque per-method declaration locations and equal-valued overrides are unavailable. Rendered text is captured at text boundaries (16 KiB UTF-8 prefix); editable control values are excluded. Bounded accessibility labels and descriptions include macro-generated control names."
            } else {
                "Inspection was compiled out; enable rake/devtools"
            }.into(),
        }
    }

    fn tree_page(&self, _: Option<&str>, _: usize) -> Result<TreePage, InspectionError> {
        Err(InspectionError::Unavailable)
    }

    fn tree_revision(&self) -> TreeRevision {
        TreeRevision {
            metadata: None,
            capture: "0".into(),
            status: Status::Unavailable,
        }
    }

    fn element(&self, _: &str) -> Result<Element, InspectionError> {
        Err(InspectionError::Unavailable)
    }

    fn pick(&self) -> Result<Pick, InspectionError> {
        Err(InspectionError::Unavailable)
    }

    fn highlight(&self, _: Option<&str>, _: &str) -> Result<(), InspectionError> {
        Err(InspectionError::Unavailable)
    }

    fn inspect_mode(&self, _: bool, _: u64) -> Result<(), InspectionError> {
        Err(InspectionError::Unavailable)
    }

    fn navigate(&self, _: &str, _: &str, _: NavigationAction) -> Result<(), InspectionError> {
        Err(InspectionError::Unavailable)
    }

    fn edit_style(&self, _: &str, _: &str, _: &StyleEdit) -> Result<(), InspectionError> {
        Err(InspectionError::Unavailable)
    }

    fn record_frames(&self, _: bool, _: u64) -> Result<(), InspectionError> {
        Err(InspectionError::RecordingUnavailable)
    }

    fn frames(&self, _: u64, _: &rake_diagnostics::Collector) -> Frames {
        Frames {
            clock: None,
            recording: false,
            frames: Vec::new(),
            cursor: 0,
            more: false,
            dropped: 0,
            gap: false,
        }
    }
}

pub(crate) struct Unavailable;

impl InspectionHost for Unavailable {}
