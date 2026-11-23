use std::panic::Location;

use crate::gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    Pixels, SharedString, Text, Window, accesskit,
};

/// Rendered text carrying its original Fluent message identifier.
#[derive(Clone, Debug)]
pub struct LocalizedText {
    key: &'static str,
    text: SharedString,
    source: &'static Location<'static>,
}

impl LocalizedText {
    #[track_caller]
    pub(super) fn new(key: &'static str, translated: Option<String>) -> Self {
        Self {
            key,
            text: translated.map_or_else(|| SharedString::new_static(key), SharedString::from),
            source: Location::caller(),
        }
    }

    /// The rendered text, for APIs taking a plain string.
    pub fn as_str(&self) -> &str {
        self.text.as_ref()
    }

    /// The catalogue message identifier.
    pub fn message_key(&self) -> &'static str {
        self.key
    }
}

impl AsRef<str> for LocalizedText {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl From<LocalizedText> for SharedString {
    fn from(value: LocalizedText) -> Self {
        value.text
    }
}

impl IntoElement for LocalizedText {
    type Element = LocalizedElement;

    fn into_element(self) -> Self::Element {
        LocalizedElement {
            inner: Text::new(ElementId::CodeLocation(*self.source), self.text),
            #[cfg(feature = "devtools")]
            key: self.key,
        }
    }
}

/// GPUI text boundary carrying the Fluent message key for inspection.
#[doc(hidden)]
pub struct LocalizedElement {
    inner: Text,
    #[cfg(feature = "devtools")]
    pub(crate) key: &'static str,
}

impl IntoElement for LocalizedElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for LocalizedElement {
    type RequestLayoutState = <Text as Element>::RequestLayoutState;
    type PrepaintState = <Text as Element>::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        Element::id(&self.inner)
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        self.inner.source_location()
    }

    fn a11y_role(&self) -> Option<accesskit::Role> {
        self.inner.a11y_role()
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        self.inner.write_a11y_info(node);
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.inner.request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.inner
            .prepaint(id, inspector_id, bounds, state, window, cx)
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner
            .paint(id, inspector_id, bounds, state, prepaint, window, cx);
    }
}
