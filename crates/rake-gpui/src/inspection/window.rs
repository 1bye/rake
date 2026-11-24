//! Capture Kit's presentation layers and application content in one window tree.

use std::collections::HashMap;
use std::rc::Rc;

use gpui::{AnyElement, App, Context, Global, IntoElement, Render, Window};
use rake_ui::kit::base::{Root, RootPlugin};

use super::Store;
use crate::router::WindowId;

struct WindowCapture {
    store: Store,
    window: WindowId,
    revision: Rc<dyn Fn() -> u64>,
}

#[derive(Default)]
struct Pending(HashMap<gpui::WindowId, WindowCapture>);

impl Global for Pending {}

pub(crate) fn prepare(
    window: &Window,
    cx: &mut App,
    store: Store,
    host: WindowId,
    revision: impl Fn() -> u64 + 'static,
) {
    if !cx.has_global::<Pending>() {
        cx.set_global(Pending::default());
    }

    cx.global_mut::<Pending>().0.insert(
        window.window_handle().window_id(),
        WindowCapture {
            store,
            window: host,
            revision: Rc::new(revision),
        },
    );
    Root::register_plugin::<InspectionLayer>(cx, |window, cx| {
        InspectionLayer(
            cx.global_mut::<Pending>()
                .0
                .remove(&window.window_handle().window_id()),
        )
    });
}

struct InspectionLayer(Option<WindowCapture>);

impl Render for InspectionLayer {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

impl RootPlugin for InspectionLayer {
    fn decorate(
        &self,
        surface: AnyElement,
        _: &Root,
        _: &mut Window,
        _: &mut App,
    ) -> impl IntoElement {
        if let Some(capture) = &self.0 {
            super::view::capture_window(
                surface,
                capture.store.clone(),
                (capture.revision)(),
                capture.window,
            )
        } else {
            surface
        }
    }
}

#[cfg(all(test, feature = "view-test-support"))]
mod tests {
    //! Covers: Kit global dialog boundaries sharing the application inspection capture.
    //! Real: Base root plugins, Kit dialog presentation and Rake inspection.
    //! Doubles: TestPlatform; no display or GPU.

    use super::*;
    use crate::inspection::InspectElement as _;
    use gpui::{AppContext as _, InteractiveElement as _, ParentElement as _, Styled as _};
    use rake_ui::kit::component::WindowExt as _;

    struct Content(Store);

    impl Render for Content {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            gpui::div()
                .size_full()
                .inspect("page-content")
                .root(self.0.clone(), 7, WindowId(1))
        }
    }

    #[gpui::test]
    fn kit_dialog_and_page_boundaries_share_one_window_capture(cx: &mut gpui::TestAppContext) {
        let store = Store::default();

        store.attach();
        cx.update(rake_ui::init);
        let (_, cx) = cx.add_window_view(|window, cx| {
            prepare(window, cx, store.clone(), WindowId(1), || 7);
            let content = cx.new(|_| Content(store.clone()));

            Root::new(content, window, cx)
        });

        cx.update(|window, cx| {
            window.open_dialog(cx, |dialog, _, _| {
                dialog.title("Kit dialog").child(
                    gpui::div()
                        .child("Global dialog content")
                        .child(crate::view! {
                            <button id="kit-dialog-rake-button">"Inside Kit dialog"</button>
                        })
                        .inspect("kit.dialog.content"),
                )
            });
            window.draw(cx).clear(cx);
        });
        let snapshot = store.snapshot();

        for name in ["rake.window", "page-content", "kit.dialog.content"] {
            assert!(
                snapshot
                    .elements
                    .iter()
                    .any(|element| element.semantic_id.as_deref() == Some(name)),
                "missing boundary {name}"
            );
        }

        assert_eq!(snapshot.router_revision, 7);
        let content = snapshot
            .elements
            .iter()
            .find(|element| element.semantic_id.as_deref() == Some("kit.dialog.content"))
            .unwrap();

        assert!(
            content.paint_order.is_some(),
            "capture includes deferred paint, not only layout"
        );
        let button = snapshot
            .elements
            .iter()
            .find(|element| element.semantic_id.as_deref() == Some("kit-dialog-rake-button"))
            .unwrap();

        assert!(
            button.style_editable,
            "Rake content stays editable inside Kit's dialog"
        );
        let stable = content.stable_id.clone();

        store
            .highlight(Some(&content.id), &snapshot.capture)
            .unwrap();
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
        assert!(cx.debug_bounds("rake.inspection.highlight").is_some());
        let next = store.snapshot();

        assert_eq!(
            next.elements
                .iter()
                .find(|element| element.semantic_id.as_deref() == Some("kit.dialog.content"))
                .unwrap()
                .stable_id,
            stable
        );
    }
    struct MovingContent {
        store: Store,
        left: Rc<std::cell::Cell<f32>>,
    }

    impl Render for MovingContent {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            gpui::div()
                .size_full()
                .children((self.left.get() >= 0.).then(|| {
                    gpui::div()
                        .id("moving")
                        .absolute()
                        .left(gpui::px(self.left.get()))
                        .size(gpui::px(40.))
                        .inspect("moving-boundary")
                }))
                .inspect("moving-page")
                .root(self.store.clone(), 0, WindowId(1))
        }
    }

    #[gpui::test]
    fn window_highlight_tracks_current_frame_geometry(cx: &mut gpui::TestAppContext) {
        let store = Store::default();
        store.attach();
        cx.update(rake_ui::init);
        let left = Rc::new(std::cell::Cell::new(10.));
        let (_, cx) = cx.add_window_view(|window, cx| {
            prepare(window, cx, store.clone(), WindowId(1), || 0);
            let content = cx.new(|_| MovingContent {
                store: store.clone(),
                left: left.clone(),
            });
            Root::new(content, window, cx)
        });
        let draw = |cx: &mut gpui::VisualTestContext| {
            cx.update(|window, cx| {
                window.refresh();
                window.draw(cx).clear(cx);
            })
        };
        draw(cx);
        let snapshot = store.snapshot();
        let target = snapshot
            .elements
            .iter()
            .find(|e| e.semantic_id.as_deref() == Some("moving-boundary"))
            .unwrap();
        store
            .highlight(Some(&target.id), &snapshot.capture)
            .unwrap();
        left.set(90.);
        draw(cx);
        let highlight = cx
            .debug_bounds("rake.inspection.highlight")
            .expect("highlight painted");
        assert_eq!(
            highlight.origin.x,
            gpui::px(90.),
            "highlight must follow this frame's layout"
        );
        left.set(-1.);
        draw(cx);
        assert!(
            cx.debug_bounds("rake.inspection.highlight").is_none(),
            "removed targets must not leave ghost highlights"
        );
    }
}
