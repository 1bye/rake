//! End window capture after GPUI has painted every deferred presentation layer.

use std::{cell::RefCell, panic::Location, rc::Rc};

use super::{
    ActiveCapture, Capture, CapturingLayout, LayoutContext, LayoutHost, Status, Store, inspect,
    paint_overlay, publish_capture,
};

use crate::router::WindowId;

use gpui::{
    App, Bounds, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId, Pixels,
    Styled as _, Window,
};

use gpui::{InteractiveElement as _, ParentElement as _};

#[derive(Default)]
pub(super) struct Completion {
    pub(super) capture: Option<Option<Capture>>,
    pub(super) host: Option<LayoutContext>,
    pub(super) layout: Option<bool>,
    pub(super) overlay: Option<gpui::AnyElement>,
}

pub(in crate::inspection) fn capture(
    surface: gpui::AnyElement,
    store: Store,
    revision: u64,
    host: WindowId,
) -> gpui::AnyElement {
    if store.status() == Status::Unavailable {
        return surface;
    }

    let completion = Rc::new(RefCell::new(Completion::default()));

    // GPUI sorts all deferred paint, including nested draws, by priority.
    // The last sentinel publishes after both application and Kit overlays.
    let end = gpui::deferred(EndCapture {
        store: store.clone(),
        host,
        completion: completion.clone(),
    })
    .with_priority(usize::MAX);

    let surface = gpui::div()
        .id("rake.window")
        .size_full()
        .child(surface)
        .child(end);

    let mut boundary = inspect(surface)
        .semantic_id("rake.window")
        .root(store, revision, host);

    boundary.window_finish = Some(completion);

    boundary.into_any_element()
}

struct EndCapture {
    store: Store,
    host: WindowId,
    completion: Rc<RefCell<Completion>>,
}

impl IntoElement for EndCapture {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl gpui::Element for EndCapture {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (
            window.request_layout(
                gpui::Style {
                    position: gpui::Position::Absolute,
                    ..Default::default()
                },
                [],
                cx,
            ),
            (),
        )
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let (previous, layout, host) = {
            let mut completion = self.completion.borrow_mut();

            (
                completion
                    .capture
                    .take()
                    .expect("window capture prepainted"),
                completion.layout.take().expect("window capture laid out"),
                completion.host.take(),
            )
        };

        let completed = std::mem::replace(&mut cx.global_mut::<ActiveCapture>().0, previous);

        cx.global_mut::<CapturingLayout>().0 = layout;
        cx.global_mut::<LayoutHost>().0 = host;

        if let Some(mut completed) = completed {
            let identities = completed.identify(&self.store, self.host);

            for element in &mut completed.snapshot.elements {
                element.style_editable &= element.stable_id.is_some();
                element.text_editable &= element.stable_id.is_some();

                if element.stable_id.is_none() {
                    element.style_overrides.clear();
                }
            }

            publish_capture(&self.store, self.host, completed, identities, window, cx);
        }

        if let Some(mut overlay) = self.completion.borrow_mut().overlay.take() {
            paint_overlay(&self.store, self.host, &mut overlay, window, cx);
        }
    }
}
