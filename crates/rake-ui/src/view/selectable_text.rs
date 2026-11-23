// Copyright 2024 - 2026 Longbridge <https://longbridge.com>
// SPDX-License-Identifier: Apache-2.0
// Adapted from longbridge/gpui-kit crates/base (gpui-base 0.9.0).
// Rake accepts pre-styled runs and makes selection registration optional.
// The selection engine stays in Base. See LICENSE-GPUI-BASE.

use std::ops::Range;

use gpui::{
    App, BorderStyle, Bounds, Corners, Edges, Element, ElementId, GlobalElementId, HitboxBehavior,
    Hsla, InspectorElementId, IntoElement, LayoutId, PaintQuad, Pixels, Point, SharedString,
    StyledText, Window, transparent_black,
};

use gpui_base::{TextSelectionHandle, TextSelectionRegistration, TextSelectionRun};

pub(super) struct SelectableText {
    text: SharedString,
    styled_text: StyledText,
    selectable: bool,
}

impl SelectableText {
    pub(super) fn new(text: SharedString, styled_text: StyledText, selectable: bool) -> Self {
        Self {
            text,
            styled_text,
            selectable,
        }
    }

    fn paint_selection(
        layout: &gpui::TextLayout,
        range: Range<usize>,
        color: Hsla,
        window: &mut Window,
    ) {
        let (Some(start), Some(end)) = (
            layout.position_for_index(range.start),
            layout.position_for_index(range.end),
        ) else {
            return;
        };

        for bounds in selection_quad_bounds(start, end, layout.bounds(), layout.line_height()) {
            window.paint_quad(PaintQuad {
                bounds,
                background: color.into(),
                corner_radii: Corners::default(),
                border_widths: Edges::default(),
                border_color: transparent_black(),
                border_style: BorderStyle::default(),
            });
        }
    }
}

fn selection_quad_bounds(
    start: Point<Pixels>,
    end: Point<Pixels>,
    bounds: Bounds<Pixels>,
    line_height: Pixels,
) -> Vec<Bounds<Pixels>> {
    if start.y == end.y {
        return vec![Bounds::from_corners(
            start,
            Point::new(end.x, end.y + line_height),
        )];
    }

    let mut quads = vec![Bounds::from_corners(
        start,
        Point::new(bounds.right(), start.y + line_height),
    )];

    if end.y > start.y + line_height {
        quads.push(Bounds::from_corners(
            Point::new(bounds.left(), start.y + line_height),
            Point::new(bounds.right(), end.y),
        ));
    }

    quads.push(Bounds::from_corners(
        Point::new(bounds.left(), end.y),
        Point::new(end.x, end.y + line_height),
    ));

    quads
}

impl IntoElement for SelectableText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SelectableText {
    type RequestLayoutState = Option<TextSelectionHandle>;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some("selection".into())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let selection = self.selectable.then(|| {
            window.with_element_state(
                global_id.expect("selectable paragraph must have a stable element id"),
                |retained: Option<TextSelectionHandle>, _| {
                    let handle =
                        retained.unwrap_or_else(|| TextSelectionHandle::new(self.text.clone(), cx));

                    (handle.clone(), handle)
                },
            )
        });

        let (layout_id, ()) = self
            .styled_text
            .request_layout(global_id, inspector_id, window, cx);

        (layout_id, selection)
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        selection: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.styled_text
            .prepaint(global_id, inspector_id, bounds, &mut (), window, cx);

        if let Some(handle) = selection {
            let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
            let registration = TextSelectionRegistration::new(hitbox, bounds)
                .with_text_bounds(vec![bounds])
                .with_rendered_element(handle, window, cx);

            handle.register(registration, window, cx);
        }
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        selection: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(handle) = selection {
            let layout = self.styled_text.layout();
            let projection = handle.update_runs(
                &[TextSelectionRun::new(
                    self.text.clone(),
                    layout.clone(),
                    bounds,
                )],
                cx,
            );

            // Base caches the copy text while projecting the run.
            if let Some(Some(range)) = projection.ranges().first() {
                let color = gpui_base::Theme::global(cx).tokens.colors.selection;

                Self::paint_selection(layout, range.clone(), color, window);
            }
        }

        self.styled_text.paint(
            global_id,
            inspector_id,
            bounds,
            &mut (),
            &mut (),
            window,
            cx,
        );
    }
}

#[cfg(all(test, feature = "test-support", not(target_family = "wasm")))]
mod tests {
    use gpui::{
        Context, InteractiveElement as _, IntoElement, Modifiers, MouseButton, ParentElement as _,
        Render, SharedString, Styled as _, StyledText, TestAppContext, Window, div, point, px,
    };

    use gpui_base::{TextSelection, TextSelectionLayer};

    use super::SelectableText;

    struct MixedSelection;

    impl Render for MixedSelection {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let text = |value, selectable| {
                let value = SharedString::from(value);
                let styled = StyledText::new(value.clone());

                SelectableText::new(value, styled, selectable)
            };

            div().size_full().child(TextSelectionLayer).child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(240.))
                    .line_height(px(20.))
                    .child(text("Rake", true))
                    .child(div().id("excluded").child(text("Excluded", false)))
                    .child(gpui_base::SelectableText::new("base", "Base")),
            )
        }
    }

    #[test]
    fn rake_and_base_share_window_selection() {
        let mut cx = TestAppContext::single();

        let (_, cx) = cx.add_window_view(|_, cx| {
            crate::init(cx);

            MixedSelection
        });

        cx.update(|window, cx| window.draw(cx).clear(cx));
        let start = point(px(1.), px(8.));
        let end = point(px(200.), px(45.));

        cx.simulate_mouse_move(start, None, Modifiers::default());
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::default());
        cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());

        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            let selected = TextSelection::selected_text(window, cx);
            let mut runs: Vec<_> = selected.lines().collect();

            runs.sort_unstable();
            assert_eq!(runs, ["Base", "Rake"]);
        });
    }
}
