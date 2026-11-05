//! Bridges AppKit text callbacks into the independent native renderer.

use super::Bridge;
use rake::gpui::{
    App, Bounds, InputHandler, Pixels, Point, UTF16Selection, Window, point, px, size,
};
use std::{ops::Range, rc::Rc};

pub(super) struct ImeInput {
    pub bridge: Rc<Bridge>,
    pub viewport: Bounds<Pixels>,
}

impl InputHandler for ImeInput {
    fn accepts_text_input(&mut self, window: &mut Window, _: &mut App) -> bool {
        self.bridge.accepts_text(window)
    }

    fn prefers_ime_for_printable_keys(&mut self, window: &mut Window, _: &mut App) -> bool {
        self.bridge.accepts_text(window)
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        window: &mut Window,
        _: &mut App,
    ) -> Option<UTF16Selection> {
        self.bridge.accepts_text(window).then(|| UTF16Selection {
            range: self.bridge.controller.text().selection(),
            reversed: false,
        })
    }

    fn marked_text_range(&mut self, window: &mut Window, _: &mut App) -> Option<Range<usize>> {
        if !self.bridge.accepts_text(window) {
            return None;
        }

        self.bridge.controller.text().marked_range()
    }

    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        window: &mut Window,
        _: &mut App,
    ) -> Option<String> {
        if !self.bridge.accepts_text(window) {
            return None;
        }

        let snapshot = self.bridge.controller.text();
        let (text, actual) = snapshot.text_for_range(range);
        *adjusted = Some(actual);
        Some(text.to_owned())
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        _: &mut App,
    ) {
        if self.bridge.accepts_text(window) {
            self.bridge.controller.replace_text(range, text);
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selection: Option<Range<usize>>,
        window: &mut Window,
        _: &mut App,
    ) {
        if self.bridge.accepts_text(window) {
            self.bridge.controller.mark_text(range, text, selection);
        }
    }

    fn unmark_text(&mut self, window: &mut Window, _: &mut App) {
        if self.bridge.accepts_text(window) {
            self.bridge.controller.unmark_text();
        }
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        window: &mut Window,
        _: &mut App,
    ) -> Option<Bounds<Pixels>> {
        // shortcut: anchor candidates to the visible viewport until the producer draws a caret.
        self.bridge.accepts_text(window).then(|| {
            Bounds::new(
                self.viewport.origin + point(px(8.), px(8.)),
                size(px(2.), px(20.)),
            )
        })
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut App,
    ) -> Option<usize> {
        None
    }
}
