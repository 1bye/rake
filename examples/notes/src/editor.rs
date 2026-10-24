//! Small plain-text editor using GPUI shaping/input APIs, with Unicode selection,
//! clipboard, bounded undo/redo, wrapping and IME marked text.

use rake::gpui::{self, prelude::*, *};
use std::ops::Range;
use std::rc::Rc;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone)]
struct Edit {
    text: String,
    anchor: usize,
    cursor: usize,
}

#[derive(Default)]
struct Buffer {
    text: String,
    anchor: usize,
    cursor: usize,
    marked: Option<Range<usize>>,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
}

impl Buffer {
    fn selection(&self) -> Range<usize> {
        self.anchor.min(self.cursor)..self.anchor.max(self.cursor)
    }

    fn snapshot(&self) -> Edit {
        Edit {
            text: self.text.clone(),
            anchor: self.anchor,
            cursor: self.cursor,
        }
    }

    fn restore(&mut self, edit: Edit) {
        self.text = edit.text;
        self.anchor = edit.anchor;
        self.cursor = edit.cursor;
        self.marked = None;
    }

    fn replace(&mut self, range: Range<usize>, text: &str, record: bool) {
        if record {
            self.undo.push(self.snapshot());
            self.redo.clear();
        }

        while self.undo.len() > 100
            || self.undo.iter().map(|edit| edit.text.len()).sum::<usize>() > 2 * 1024 * 1024
        {
            self.undo.remove(0);
        }

        self.text.replace_range(range.clone(), text);
        self.cursor = range.start + text.len();
        self.anchor = self.cursor;
    }

    fn previous(&self, offset: usize) -> usize {
        self.text
            .grapheme_indices(true)
            .rev()
            .find_map(|(i, _)| (i < offset).then_some(i))
            .unwrap_or(0)
    }

    fn next(&self, offset: usize) -> usize {
        self.text
            .grapheme_indices(true)
            .find_map(|(i, _)| (i > offset).then_some(i))
            .unwrap_or(self.text.len())
    }

    fn move_to(&mut self, index: usize, extend: bool) {
        self.cursor = index.min(self.text.len());

        if !extend {
            self.anchor = self.cursor;
        }

        self.marked = None;
    }

    fn undo(&mut self, redo: bool) {
        let edit = if redo {
            self.redo.pop()
        } else {
            self.undo.pop()
        };

        if let Some(edit) = edit {
            let current = self.snapshot();

            if redo {
                self.undo.push(current);
            } else {
                self.redo.push(current);
            }

            self.restore(edit);
        }
    }
}

fn utf16(text: &str, offset: usize) -> usize {
    text[..offset.min(text.len())].encode_utf16().count()
}

fn utf8(text: &str, offset: usize) -> usize {
    let mut count = 0;

    for (index, ch) in text.char_indices() {
        if count + ch.len_utf16() > offset {
            return index;
        }

        count += ch.len_utf16();
    }

    text.len()
}

fn from_utf16(text: &str, range: Range<usize>) -> Range<usize> {
    utf8(text, range.start.min(range.end))..utf8(text, range.end.max(range.start))
}

fn to_utf16(text: &str, range: Range<usize>) -> Range<usize> {
    utf16(text, range.start)..utf16(text, range.end)
}

type Changed = Rc<dyn Fn(&str, &str) -> Result<(), String>>;

#[derive(Clone)]
struct Line {
    start: usize,
    line: Rc<WrappedLine>,
    origin: Point<Pixels>,
}

pub(crate) struct Editor {
    buffer: Buffer,
    focus: FocusHandle,
    placeholder: String,
    label: String,
    size: f32,
    multiline: bool,
    changed: Changed,
    lines: Vec<Line>,
    line_height: Pixels,
    bounds: Bounds<Pixels>,
    scroll: ScrollHandle,
    dragging: bool,
    error: Option<String>,
    reveal_cursor: bool,
}

impl Editor {
    pub(crate) fn new(
        placeholder: &str,
        label: &str,
        size: f32,
        multiline: bool,
        changed: impl Fn(&str, &str) -> Result<(), String> + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            buffer: Buffer::default(),
            focus: cx.focus_handle().tab_stop(true),
            placeholder: placeholder.into(),
            label: label.into(),
            size,
            multiline,
            changed: Rc::new(changed),
            lines: Vec::new(),
            line_height: px(size * 1.8),
            bounds: Bounds::default(),
            scroll: ScrollHandle::new(),
            dragging: false,
            error: None,
            reveal_cursor: false,
        }
    }

    pub(crate) fn set_text(&mut self, text: String, reset: bool, cx: &mut Context<Self>) {
        if self.buffer.text == text && !reset {
            return;
        }

        self.buffer.text = text;

        if reset {
            self.buffer.anchor = 0;
            self.buffer.cursor = 0;
            self.buffer.undo.clear();
            self.buffer.redo.clear();
            self.scroll.set_offset(point(px(0.), px(0.)));
        } else {
            self.buffer.cursor = self.buffer.cursor.min(self.buffer.text.len());

            while !self.buffer.text.is_char_boundary(self.buffer.cursor) {
                self.buffer.cursor -= 1;
            }

            self.buffer.anchor = self.buffer.cursor;
        }

        self.buffer.marked = None;
        cx.notify();
    }

    fn commit(&mut self, before: String, cx: &mut Context<Self>) {
        self.reveal_cursor = true;

        if before != self.buffer.text {
            if let Err(error) = (self.changed)(&before, &self.buffer.text) {
                self.error = Some(error);
                self.buffer.text = before;
                self.buffer.cursor = self.buffer.cursor.min(self.buffer.text.len());

                while !self.buffer.text.is_char_boundary(self.buffer.cursor) {
                    self.buffer.cursor -= 1;
                }

                self.buffer.anchor = self.buffer.cursor;
                self.buffer.marked = None;
            } else {
                self.error = None;
            }
        }

        cx.notify();
    }

    fn replace(
        &mut self,
        range: Range<usize>,
        text: &str,
        cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        let text = if self.multiline {
            text.to_string()
        } else {
            text.replace(['\n', '\r'], " ")
        };

        if self.buffer.text.len() - (range.end - range.start) + text.len() > 512 * 1024 {
            self.error = Some(
                "This note has reached its editing limit. Copy or export it before adding more."
                    .into(),
            );
            cx.notify();

            return None;
        }

        let before = self.buffer.text.clone();
        let record = self.buffer.marked.is_none();
        let inserted = range.start..range.start + text.len();

        self.buffer.replace(range, &text, record);
        self.buffer.marked = None;
        self.commit(before, cx);

        self.error.is_none().then_some(inserted)
    }

    fn index(&self, position: Point<Pixels>) -> usize {
        for line in &self.lines {
            let height = line.line.size(self.line_height).height;

            if position.y < line.origin.y + height {
                let relative = position - line.origin;

                return (line.start
                    + line
                        .line
                        .closest_index_for_position(
                            point(relative.x.max(px(0.)), relative.y.max(px(0.))),
                            self.line_height,
                        )
                        .unwrap_or_else(|index| index)
                        .min(line.line.len()))
                .min(self.buffer.text.len());
            }
        }

        self.buffer.text.len()
    }

    fn position(&self, index: usize) -> Option<Point<Pixels>> {
        self.lines.iter().find_map(|line| {
            (index >= line.start && index <= line.start + line.line.len())
                .then(|| {
                    line.line
                        .position_for_index(index - line.start, self.line_height)
                        .map(|position| line.origin + position)
                })
                .flatten()
        })
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let modifiers = event.keystroke.modifiers;
        let command = modifiers.platform || modifiers.control;
        let key = event.keystroke.key.as_str();
        let selection = self.buffer.selection();

        // Browser clipboard contents arrive through the DOM paste event; consuming
        // its keydown would prevent the platform input handler from receiving it.
        if cfg!(target_family = "wasm") && command && key == "v" {
            return;
        }

        if command && matches!(key, "a" | "c" | "x" | "v" | "z") {
            match key {
                "a" => {
                    self.buffer.anchor = 0;
                    self.buffer.cursor = self.buffer.text.len();
                }
                "c" | "x" => {
                    if !selection.is_empty() {
                        cx.write_to_clipboard(ClipboardItem::new_string(
                            self.buffer.text[selection.clone()].into(),
                        ));
                    }

                    if key == "x" {
                        self.replace(selection, "", cx);
                    }
                }
                "v" => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                        self.replace(selection, &text, cx);
                    }
                }
                "z" => {
                    let before = self.buffer.text.clone();

                    self.buffer.undo(modifiers.shift);
                    self.commit(before, cx);
                }
                _ => {}
            }
        } else if matches!(key, "left" | "right" | "up" | "down" | "home" | "end") {
            let cursor = self.buffer.cursor;

            let next = match key {
                "left" if command => self.buffer.text[..cursor]
                    .rfind('\n')
                    .map_or(0, |index| index + 1),
                "right" if command => self.buffer.text[cursor..]
                    .find('\n')
                    .map_or(self.buffer.text.len(), |index| cursor + index),
                "left" if modifiers.alt || modifiers.control => self.buffer.text[..cursor]
                    .unicode_word_indices()
                    .next_back()
                    .map_or(0, |(index, _)| index),
                "right" if modifiers.alt || modifiers.control => self.buffer.text[cursor..]
                    .unicode_word_indices()
                    .next()
                    .map_or(self.buffer.text.len(), |(index, word)| {
                        cursor + index + word.len()
                    }),
                "left" if !modifiers.shift && !selection.is_empty() => selection.start,
                "right" if !modifiers.shift && !selection.is_empty() => selection.end,
                "left" => self.buffer.previous(cursor),
                "right" => self.buffer.next(cursor),
                "home" => 0,
                "end" => self.buffer.text.len(),
                "up" | "down" => self
                    .position(cursor)
                    .map(|position| {
                        self.index(
                            position
                                + point(
                                    px(0.),
                                    if key == "up" {
                                        -self.line_height
                                    } else {
                                        self.line_height
                                    },
                                ),
                        )
                    })
                    .unwrap_or(cursor),
                _ => cursor,
            };

            self.buffer.move_to(next, modifiers.shift);
        } else if key == "backspace" || key == "delete" {
            let range = if selection.is_empty() {
                if key == "backspace" {
                    self.buffer.previous(selection.start)..selection.end
                } else {
                    selection.start..self.buffer.next(selection.end)
                }
            } else {
                selection
            };

            self.replace(range, "", cx);
        } else if key == "enter" && self.multiline {
            self.replace(selection, "\n", cx);
        } else {
            return;
        }

        cx.stop_propagation();
        self.reveal_cursor = true;
        cx.notify();

        // Keep keyboard navigation in view; a subsequent layout refines newly inserted lines.
        if let Some(position) = self.position(self.buffer.cursor) {
            let bounds = self.scroll.bounds();
            let mut offset = self.scroll.offset();

            if position.y < bounds.top() {
                offset.y += bounds.top() - position.y;
            }

            if position.y + self.line_height > bounds.bottom() {
                offset.y -= position.y + self.line_height - bounds.bottom();
            }

            self.scroll.set_offset(offset);
        }

        let _ = window;
    }
}

impl Focusable for Editor {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EntityInputHandler for Editor {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = from_utf16(&self.buffer.text, range);

        *actual = Some(to_utf16(&self.buffer.text, range.clone()));

        Some(self.buffer.text[range].into())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: to_utf16(&self.buffer.text, self.buffer.selection()),
            reversed: self.buffer.cursor < self.buffer.anchor,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.buffer
            .marked
            .clone()
            .map(|range| to_utf16(&self.buffer.text, range))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.buffer.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .map(|range| from_utf16(&self.buffer.text, range))
            .or_else(|| self.buffer.marked.clone())
            .unwrap_or_else(|| self.buffer.selection());

        self.replace(range, text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selection: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .map(|range| from_utf16(&self.buffer.text, range))
            .or_else(|| self.buffer.marked.clone())
            .unwrap_or_else(|| self.buffer.selection());

        let start = range.start;

        if let Some(inserted) = self.replace(range, text, cx) {
            if let Some(selection) = selection {
                let text = &self.buffer.text[inserted.clone()];

                self.buffer.anchor = start + utf8(text, selection.start);
                self.buffer.cursor = start + utf8(text, selection.end);
            }

            self.buffer.marked = Some(inserted);
        }

        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let start = utf8(&self.buffer.text, range.start);
        let position = self.position(start)?;

        Some(Bounds::new(position, size(px(2.), self.line_height)))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(utf16(&self.buffer.text, self.index(point)))
    }
}

impl Render for Editor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("text-editor")
            .role(if self.multiline {
                gpui::accesskit::Role::MultilineTextInput
            } else {
                gpui::accesskit::Role::TextInput
            })
            .aria_label(self.label.clone())
            .aria_value(self.buffer.text.clone())
            .aria_placeholder(self.placeholder.clone())
            .tab_index(0)
            .size_full()
            .min_h_0()
            .key_context("FieldnotesEditor")
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .text_size(px(self.size))
            .line_height(px(self.size * if self.multiline { 1.85 } else { 1.4 }))
            .text_color(rgb(0x353a37))
            .on_key_down(cx.listener(Self::key))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus, cx);
                    let index = this.index(event.position);

                    this.buffer.move_to(index, event.modifiers.shift);

                    if event.click_count >= 3 {
                        this.buffer.anchor = 0;
                        this.buffer.cursor = this.buffer.text.len();
                    } else if event.click_count == 2 {
                        let word = this
                            .buffer
                            .text
                            .unicode_word_indices()
                            .find(|(start, word)| index >= *start && index <= start + word.len());

                        if let Some((start, word)) = word {
                            this.buffer.anchor = start;
                            this.buffer.cursor = start + word.len();
                        }
                    }

                    this.dragging = true;
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if this.dragging && event.dragging() {
                    this.buffer.cursor = this.index(event.position);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.dragging = false),
            )
            .child(Text {
                editor: cx.entity(),
            })
            .children(self.error.clone().map(|error| {
                div()
                    .text_size(px(11.))
                    .text_color(rgb(0xad4b39))
                    .child(error)
            }))
    }
}

struct Text {
    editor: Entity<Editor>,
}

struct Painted {
    lines: Vec<Line>,
    cursor: Option<PaintQuad>,
    selection: Vec<PaintQuad>,
}

impl IntoElement for Text {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

fn shape(text: &str, width: Pixels, style: &TextStyle, window: &mut Window) -> Vec<WrappedLine> {
    let size = style.font_size.to_pixels(window.rem_size());

    let text: SharedString = if text.is_empty() {
        " ".into()
    } else {
        text.to_string().into()
    };

    let run = TextRun {
        len: text.len(),
        font: style.font(),
        color: style.color,
        background_color: None,
        underline: None,
        strikethrough: None,
        letter_spacing: None,
    };

    window
        .text_system()
        .shape_text(text, size, &[run], Some(width.max(px(8.))), None)
        .map(|lines| lines.into_vec())
        .unwrap_or_default()
}

impl Element for Text {
    type RequestLayoutState = ();
    type PrepaintState = Painted;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let editor = self.editor.clone();
        // Measured layout runs after the parent's text-style scope has ended.
        // Capture it now so scroll extent and painted rows use the same metrics.
        let text_style = window.text_style();
        let line_height = text_style.line_height_in_pixels(window.rem_size());
        let mut style = Style::default();

        style.size.width = relative(1.).into();
        style.flex_shrink = 0.;
        let id = window.request_measured_layout(style, move |known, available, window, cx| {
            let width = known.width.unwrap_or(match available.width {
                AvailableSpace::Definite(width) => width,
                _ => px(600.),
            });

            let content = &editor.read(cx).buffer.text;

            let height = shape(content, width, &text_style, window)
                .iter()
                .map(|line| line.size(line_height).height)
                .fold(px(0.), |a, b| a + b)
                .max(line_height);

            size(width, height + px(12.))
        });

        let _ = cx;

        (id, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Painted {
        let editor = self.editor.read(cx);
        let text = editor.buffer.text.clone();
        let selection = editor.buffer.selection();
        let cursor = editor.buffer.cursor;
        let focused = editor.focus.is_focused(window);
        let placeholder = editor.placeholder.clone();

        let display = if text.is_empty() {
            placeholder.as_str()
        } else {
            text.as_str()
        };

        let line_height = window.line_height();
        let mut lines = Vec::new();
        let mut offset = 0;
        let mut y = bounds.top();
        let mut quads = Vec::new();
        let mut caret = None;

        let mut text_style = window.text_style();

        if text.is_empty() {
            text_style.color = hsla(0.25, 0.02, 0.6, 1.);
        }

        let shaped = shape(display, bounds.size.width, &text_style, window);

        for line in shaped {
            let origin = point(bounds.left(), y);
            let len = if text.is_empty() { 0 } else { line.len() };

            if focused
                && cursor >= offset
                && cursor <= offset + len
                && caret.is_none()
                && let Some(position) = line.position_for_index(cursor - offset, line_height)
            {
                caret = Some(fill(
                    Bounds::new(origin + position, size(px(1.5), line_height)),
                    rgb(0x4b6b57),
                ));
            }

            if !selection.is_empty() && !text.is_empty() {
                let start = selection.start.saturating_sub(offset).min(len);
                let end = selection.end.saturating_sub(offset).min(len);

                if end > start {
                    let starts = std::iter::once(0)
                        .chain(line.wrap_boundaries().iter().map(|boundary| {
                            line.runs()[boundary.run_ix].glyphs[boundary.glyph_ix].index
                        }))
                        .collect::<Vec<_>>();

                    for (row, row_start) in starts.iter().copied().enumerate() {
                        let row_end = starts.get(row + 1).copied().unwrap_or(len);
                        let a = start.max(row_start);
                        let b = end.min(row_end);

                        if b > a {
                            let base = line.unwrapped_layout.x_for_index(row_start);
                            let x1 = line.unwrapped_layout.x_for_index(a) - base;
                            let x2 = line.unwrapped_layout.x_for_index(b) - base;

                            quads.push(fill(
                                Bounds::new(
                                    origin + point(x1.min(x2), line_height * row as f32),
                                    size((x2.max(x1) - x1.min(x2)).max(px(2.)), line_height),
                                ),
                                rgb(0xdbe6d8),
                            ));
                        }
                    }
                }
            }

            y += line.size(line_height).height;
            offset += len + 1;
            lines.push(Line {
                start: offset - len - 1,
                line: Rc::new(line),
                origin,
            });
        }

        self.editor.update(cx, |editor, cx| {
            editor.lines = lines.clone();
            editor.line_height = line_height;
            editor.bounds = bounds;

            if editor.reveal_cursor
                && let Some(caret) = caret.as_ref()
            {
                editor.reveal_cursor = false;
                let viewport = editor.scroll.bounds();
                let before = editor.scroll.offset();
                let mut offset = before;

                if caret.bounds.top() < viewport.top() {
                    offset.y += viewport.top() - caret.bounds.top();
                } else if caret.bounds.bottom() > viewport.bottom() {
                    offset.y -= caret.bounds.bottom() - viewport.bottom();
                }

                if offset != before {
                    editor.scroll.set_offset(offset);
                    cx.notify();
                }
            }
        });

        Painted {
            lines,
            cursor: caret,
            selection: quads,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        state: &mut Painted,
        window: &mut Window,
        cx: &mut App,
    ) {
        let editor = self.editor.read(cx);
        let focus = editor.focus.clone();
        let empty = editor.buffer.text.is_empty();

        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.editor.clone()),
            cx,
        );

        for quad in state.selection.drain(..) {
            window.paint_quad(quad);
        }

        for line in &state.lines {
            let result = if empty {
                window.with_text_style(
                    Some(TextStyleRefinement {
                        color: Some(hsla(0.25, 0.02, 0.6, 1.)),
                        ..Default::default()
                    }),
                    |window| {
                        line.line.paint(
                            line.origin,
                            window.line_height(),
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        )
                    },
                )
            } else {
                line.line.paint(
                    line.origin,
                    window.line_height(),
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                )
            };

            if let Err(error) = result {
                rake::warn!(%error, "Text painting failed");
            }
        }

        if let Some(cursor) = state.cursor.take() {
            window.paint_quad(cursor);
        }
    }

    fn a11y_role(&self) -> Option<gpui::accesskit::Role> {
        Some(gpui::accesskit::Role::TextInput)
    }

    fn write_a11y_info(&self, node: &mut gpui::accesskit::Node) {
        node.set_label("Fieldnotes text editor");
    }
}

#[cfg(test)]
mod tests {
    //! Covers: grapheme editing, UTF-16 bridges and bounded undo behavior.
    //! Real: editing buffer. Doubles: none. Requires: no display.
    use super::{Buffer, from_utf16, to_utf16, utf8};

    #[test]
    fn grapheme_deletion_and_undo_preserve_unicode_and_selection() {
        let mut buffer = Buffer {
            text: "a👩🏽‍💻e\u{301}".into(),
            ..Default::default()
        };

        buffer.cursor = buffer.text.len();
        buffer.anchor = buffer.cursor;
        let previous = buffer.previous(buffer.cursor);

        buffer.replace(previous..buffer.cursor, "", true);
        assert_eq!(buffer.text, "a👩🏽‍💻");
        buffer.undo(false);
        assert_eq!(buffer.text, "a👩🏽‍💻e\u{301}");
        buffer.undo(true);
        assert_eq!(buffer.text, "a👩🏽‍💻");
    }

    #[test]
    fn utf16_ranges_never_slice_a_surrogate_or_utf8_character() {
        let text = "a😀b";

        assert_eq!(from_utf16(text, 1..3), 1..5);
        assert_eq!(to_utf16(text, 1..5), 1..3);
        assert_eq!(utf8(text, 2), 1);
        assert_eq!(utf8(text, 999), text.len());
    }
}
