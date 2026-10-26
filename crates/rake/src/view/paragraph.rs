//! Composes UTF-8 text runs and exposes the full paragraph text to accessibility.

use std::ops::Range;

mod layout;

use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, HighlightStyle, InspectorElementId,
    IntoElement, LayoutId, Pixels, SharedString, Window, accesskit,
};

/// Inherited inline-only styling, independent of paragraph layout properties.
#[doc(hidden)]
#[derive(Clone, Default, PartialEq)]
pub struct RunStyle {
    highlight: HighlightStyle,
    family: Option<SharedString>,
}

impl RunStyle {
    /// Combines a nested run's refinement with its inherited text style.
    pub fn highlight(mut self, style: HighlightStyle) -> Self {
        self.highlight = self.highlight.highlight(style);

        self
    }

    /// Overrides the inherited family without changing the paragraph's font size.
    pub fn font_family(mut self, family: impl Into<SharedString>) -> Self {
        self.family = Some(family.into());

        self
    }
}

struct Run {
    range: Range<usize>,
    style: RunStyle,
}

/// Appends selected paragraph content in source order, using valid UTF-8 byte ranges.
#[doc(hidden)]
#[derive(Default)]
pub struct ParagraphContent {
    text: String,
    runs: Vec<Run>,
}

impl ParagraphContent {
    /// Copies string-like content once, coalescing adjacent runs with identical styles.
    pub fn push(&mut self, text: impl AsRef<str>, style: &RunStyle) {
        let text = text.as_ref();

        if text.is_empty() {
            return;
        }

        let start = self.text.len();

        self.text.push_str(text);

        if let Some(last) = self.runs.last_mut()
            && last.style == *style
        {
            last.range.end = self.text.len();
        } else {
            self.runs.push(Run {
                range: start..self.text.len(),
                style: style.clone(),
            });
        }
    }

    /// Produces one GPUI text layout while preserving the untruncated accessible value.
    pub fn finish(self) -> ParagraphText {
        ParagraphText {
            text: self.text.into(),
            runs: self.runs,
            layout: Default::default(),
        }
    }
}

/// A shaped text element with one stable accessible text node inside its paragraph.
#[doc(hidden)]
pub struct ParagraphText {
    text: SharedString,
    runs: Vec<Run>,
    layout: std::rc::Rc<std::cell::RefCell<layout::ParagraphLayout>>,
}

impl IntoElement for ParagraphText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ParagraphText {
    type RequestLayoutState = std::rc::Rc<std::cell::RefCell<layout::ParagraphLayout>>;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some("rake::view:paragraph-text".into())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn a11y_role(&self) -> Option<accesskit::Role> {
        Some(accesskit::Role::Label)
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        node.set_value(self.text.to_string());
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        _cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let base = window.text_style();

        let mut runs: Vec<_> = self
            .runs
            .iter()
            .map(|run| {
                let mut text_run = base
                    .clone()
                    .highlight(run.style.highlight)
                    .to_run(run.range.len());

                if let Some(family) = &run.style.family {
                    text_run.font.family = family.clone();
                }

                text_run
            })
            .collect();

        if runs.is_empty() {
            runs.push(base.to_run(0));
        }

        let id = layout::request_layout(self.text.clone(), runs, self.layout.clone(), window);

        (id, self.layout.clone())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _state: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        layout::paint(&state.borrow(), bounds, window, cx);
    }
}

#[cfg(test)]
mod tests;
