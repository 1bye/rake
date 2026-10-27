//! Small single-line filter input with platform text/IME and clipboard handling.

use crate::appearance::Palette;
use rake::gpui::{self, prelude::*, *};
use std::ops::Range;
use std::rc::Rc;

type Changed = Rc<dyn Fn(String, &mut App)>;

pub(crate) struct FilterInput {
    text: String,
    label: &'static str,
    placeholder: &'static str,
    cursor: usize,
    anchor: usize,
    marked: Option<Range<usize>>,
    focus: FocusHandle,
    changed: Changed,
    bounds: Bounds<Pixels>,
    light: bool,
}

fn byte_index(text: &str, offset: usize) -> usize {
    let mut units = 0;

    for (index, character) in text.char_indices() {
        if units + character.len_utf16() > offset {
            return index;
        }

        units += character.len_utf16();
    }

    text.len()
}

impl FilterInput {
    pub(crate) fn new(
        label: &'static str,
        placeholder: &'static str,
        changed: impl Fn(String, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            text: String::new(),
            label,
            placeholder,
            cursor: 0,
            anchor: 0,
            marked: None,
            focus: cx.focus_handle(),
            changed: Rc::new(changed),
            bounds: Bounds::default(),
            light: true,
        }
    }

    pub(crate) fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    pub(crate) fn reset(&mut self, cx: &mut Context<Self>) {
        self.text.clear();
        self.cursor = 0;
        self.anchor = 0;
        self.marked = None;
        cx.notify();
    }

    pub(crate) fn set_light(&mut self, light: bool, cx: &mut Context<Self>) {
        self.light = light;
        cx.notify();
    }

    fn selection(&self) -> Range<usize> {
        self.anchor.min(self.cursor)..self.anchor.max(self.cursor)
    }

    fn range(&self, range: Range<usize>) -> Range<usize> {
        byte_index(&self.text, range.start.min(range.end))
            ..byte_index(&self.text, range.start.max(range.end))
    }

    fn utf16(&self, range: Range<usize>) -> Range<usize> {
        self.text[..range.start].encode_utf16().count()
            ..self.text[..range.end].encode_utf16().count()
    }

    fn replace(&mut self, range: Range<usize>, value: &str, cx: &mut Context<Self>) -> bool {
        let text: String = value
            .chars()
            .filter(|character| !character.is_control())
            .collect();

        if self.text.len() - range.len() + text.len() > 1024 {
            return false;
        }

        self.marked = None;
        self.text.replace_range(range.clone(), &text);
        self.cursor = range.start + text.len();
        self.anchor = self.cursor;
        (self.changed)(self.text.clone(), cx);
        cx.notify();

        true
    }

    fn key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;

        let command = if cfg!(target_os = "macos") {
            modifiers.platform
        } else {
            modifiers.control || modifiers.platform
        };

        let selection = self.selection();

        if cfg!(target_family = "wasm") && command && key == "v" {
            return;
        }

        if command && key == "a" {
            self.anchor = 0;
            self.cursor = self.text.len();
        } else if command && matches!(key, "c" | "x") {
            if !selection.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(
                    self.text[selection.clone()].into(),
                ));
            }

            if key == "x" {
                self.replace(selection, "", cx);
            }
        } else if command && key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                self.replace(selection, &text, cx);
            }
        } else if matches!(key, "left" | "right" | "home" | "end") {
            self.cursor = match key {
                "home" => 0,
                "end" => self.text.len(),
                "left" if !modifiers.shift && !selection.is_empty() => selection.start,
                "right" if !modifiers.shift && !selection.is_empty() => selection.end,
                "left" => self.text[..self.cursor]
                    .char_indices()
                    .next_back()
                    .map_or(0, |(index, _)| index),
                _ => self.text[self.cursor..]
                    .chars()
                    .next()
                    .map_or(self.cursor, |character| self.cursor + character.len_utf8()),
            };

            if !modifiers.shift {
                self.anchor = self.cursor;
            }
        } else if matches!(key, "backspace" | "delete") {
            let range = if !selection.is_empty() {
                selection
            } else if key == "backspace" {
                self.text[..self.cursor]
                    .char_indices()
                    .next_back()
                    .map_or(self.cursor, |(index, _)| index)..self.cursor
            } else {
                self.cursor
                    ..self.text[self.cursor..]
                        .chars()
                        .next()
                        .map_or(self.cursor, |character| self.cursor + character.len_utf8())
            };

            self.replace(range, "", cx);
        } else if key == "escape" {
            self.replace(0..self.text.len(), "", cx);
        } else {
            return;
        }

        cx.stop_propagation();
        cx.notify();
    }
}

impl EntityInputHandler for FilterInput {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range(range);

        *actual = Some(self.utf16(range.clone()));

        Some(self.text[range].into())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.utf16(self.selection()),
            reversed: self.cursor < self.anchor,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.clone().map(|range| self.utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
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
            .map(|range| self.range(range))
            .or_else(|| self.marked.take())
            .unwrap_or_else(|| self.selection());

        self.replace(range, text, cx);
        self.marked = None;
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .map(|range| self.range(range))
            .or_else(|| self.marked.take())
            .unwrap_or_else(|| self.selection());

        let start = range.start;

        if !self.replace(range, text, cx) {
            return;
        }

        let end = self.cursor;

        if let Some(selection) = selected {
            self.anchor = start + byte_index(&self.text[start..end], selection.start);
            self.cursor = start + byte_index(&self.text[start..end], selection.end);
        }

        self.marked = Some(start..end);
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(self.bounds)
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl Render for FilterInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Palette::new(self.light);
        let entity = cx.entity();
        let painted = entity.clone();
        let focus = self.focus.clone();
        let selection = self.selection();
        let focused = self.focus.is_focused(window);

        let content = if self.text.is_empty() {
            self.placeholder.into()
        } else {
            self.text.clone()
        };

        div()
            .id("filter-input")
            .relative()
            .tab_index(0)
            .track_focus(&self.focus)
            .role(gpui::accesskit::Role::TextInput)
            .aria_label(self.label)
            .aria_value(self.text.clone())
            .h(px(24.))
            .w_full()
            .px_2()
            .text_size(px(12.))
            .line_height(px(22.))
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(if focused { 0x8ab4f8 } else { colors.field }))
            .bg(rgb(colors.field))
            .text_color(rgb(if self.text.is_empty() {
                colors.muted
            } else {
                colors.ink
            }))
            .overflow_hidden()
            .on_key_down(cx.listener(Self::key))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    window.focus(&this.focus, cx);
                    cx.notify();
                }),
            )
            .child(if selection.is_empty() && focused {
                div()
                    .flex()
                    .child(self.text[..self.cursor].to_string())
                    .child(
                        div()
                            .h(px(16.))
                            .mt(px(3.))
                            .border_l_1()
                            .border_color(rgb(0x8ab4f8)),
                    )
                    .child(if self.text.is_empty() {
                        content
                    } else {
                        self.text[self.cursor..].to_string()
                    })
                    .into_any_element()
            } else if selection.is_empty() {
                content.into_any_element()
            } else {
                div()
                    .flex()
                    .child(self.text[..selection.start].to_string())
                    .child(
                        div()
                            .bg(rgb(0x3c4b64))
                            .child(self.text[selection.clone()].to_string()),
                    )
                    .child(self.text[selection.end..].to_string())
                    .into_any_element()
            })
            .child(
                canvas(
                    move |bounds, _, cx| {
                        entity.update(cx, |input, _| input.bounds = bounds);
                    },
                    move |bounds, _, window, cx| {
                        window.handle_input(&focus, ElementInputHandler::new(bounds, painted), cx);
                    },
                )
                .absolute()
                .size_full(),
            )
    }
}
