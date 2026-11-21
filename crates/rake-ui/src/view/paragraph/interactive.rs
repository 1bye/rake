//! Uses Base selection and GPUI text geometry; links occupy only their painted glyph runs.

use gpui::{
    AnyElement, App, Bounds, ElementId, FocusHandle, InteractiveElement, IntoElement, Pixels,
    Point, SharedString, StatefulInteractiveElement, Styled, StyledText, Window,
    prelude::FluentBuilder,
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ops::Range,
    rc::Rc,
};
use unicode_segmentation::UnicodeSegmentation;

use super::{ParagraphText, layout};

type LinkInspector = dyn Fn(gpui::Stateful<gpui::Div>, Option<&FocusHandle>) -> AnyElement;
type PointerGesture = Rc<Cell<Option<(Point<Pixels>, bool)>>>;

pub(super) struct Link {
    pub range: Range<usize>,
    pub href: SharedString,
    pub id: ElementId,
    pub label: Option<SharedString>,
    pub inspect: Rc<LinkInspector>,
}

#[derive(Default)]
struct Retained {
    focus: HashMap<ElementId, FocusHandle>,
    gesture: PointerGesture,
}

pub(super) struct Frame {
    text: AnyElement,
    links: Vec<AnyElement>,
    gesture: PointerGesture,
}

pub(super) fn keyboard_activates(event: &gpui::KeyDownEvent) -> bool {
    event.keystroke.key == "enter" && !event.keystroke.modifiers.modified() && !event.is_held
}

fn dragged(start: Point<Pixels>, position: Point<Pixels>) -> bool {
    (position.x - start.x).abs() > gpui::px(3.) || (position.y - start.y).abs() > gpui::px(3.)
}

impl Frame {
    pub(super) fn new(
        paragraph: &ParagraphText,
        id: &gpui::GlobalElementId,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        // Keep the paragraph's grapheme-safe truncation, including the visible affix.
        let measured = paragraph.layout.borrow();
        let visible: SharedString = measured
            .lines
            .iter()
            .map(|line| line.text.as_ref())
            .collect::<Vec<_>>()
            .join("\n")
            .into();
        let mut runs = Vec::new();
        let base = window.text_style();
        for run in &paragraph.runs {
            let mut style = base.clone().highlight(run.style.highlight);
            if let Some(family) = &run.style.family {
                style.font_family = family.clone();
            }
            if paragraph
                .links
                .iter()
                .any(|link| link.range.start < run.range.end && run.range.start < link.range.end)
            {
                style.color = gpui_base::Theme::global(cx).tokens.colors.primary;
                style.underline = Some(gpui::UnderlineStyle {
                    thickness: gpui::px(1.),
                    color: None,
                    wavy: false,
                });
            }
            runs.push(style.to_run(run.range.len()));
        }
        let prefix = visible
            .chars()
            .zip(paragraph.text.chars())
            .take_while(|(a, b)| a == b)
            .map(|(a, _)| a.len_utf8())
            .sum::<usize>();
        let runs = layout::prefix_runs(&runs, prefix, visible.len() - prefix);
        let styled = StyledText::new(visible.clone()).with_runs(runs);
        let text_layout = styled.layout().clone();
        let mut text = gpui_base::SelectableText::new("selection", visible.clone())
            .styled(styled)
            .selectable(paragraph.selectable)
            .into_any_element();
        text.layout_as_root(bounds.size.map(gpui::AvailableSpace::Definite), window, cx);
        text.prepaint_at(bounds.origin, window, cx);
        drop(measured);

        // Scope retained handles to the paragraph; link keys come from authored IDs.
        let retained = window.with_element_state(id, |state: Option<Rc<RefCell<Retained>>>, _| {
            let state = state.unwrap_or_default();
            (state.clone(), state)
        });
        let mut state = retained.borrow_mut();
        state
            .focus
            .retain(|id, _| paragraph.links.iter().any(|link| &link.id == id));
        let mut links = Vec::new();
        for link in &paragraph.links {
            let focus = state
                .focus
                .entry(link.id.clone())
                .or_insert_with(|| cx.focus_handle().tab_stop(true))
                .clone();
            let end = link.range.end.min(prefix);
            let rectangles =
                link_rectangles(&visible, link.range.start.min(end)..end, &text_layout);
            let label = link
                .label
                .clone()
                .filter(|label| !label.trim().is_empty())
                .unwrap_or_else(|| paragraph.text[link.range.clone()].to_string().into());
            let first_offset = rectangles.first().map(|(offset, _)| *offset);
            for (source_offset, rectangle) in rectangles {
                let click_focus = focus.clone();
                let activation_focus = focus.clone();
                let href = link.href.clone();
                let gesture = state.gesture.clone();
                let element = gpui::div()
                    .id((
                        link.id.clone(),
                        gpui::SharedString::from(source_offset.to_string()),
                    ))
                    .w(rectangle.size.width)
                    .h(rectangle.size.height)
                    .cursor_pointer()
                    .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                        click_focus.focus(window, cx)
                    })
                    .when(first_offset == Some(source_offset), |element| {
                        let href = href.clone();
                        element
                            .role(gpui::accesskit::Role::Link)
                            .aria_label(label.clone())
                            .track_focus(&focus)
                            .on_key_down(move |event, window, cx| {
                                if keyboard_activates(event) {
                                    super::super::link::activate(&href, window, cx);
                                    window.prevent_default();
                                    cx.stop_propagation();
                                }
                            })
                    })
                    .when(focus.is_focused(window), |element| {
                        element.bg(gpui_base::Theme::global(cx).tokens.colors.selection)
                    })
                    .on_click(move |event, window, cx| {
                        let follows = match event {
                            gpui::ClickEvent::Mouse(_) => {
                                gesture.get().is_some_and(|(_, dragged)| !dragged)
                            }
                            gpui::ClickEvent::Touch(_) => true,
                            gpui::ClickEvent::Keyboard(_) => false,
                        };
                        if follows {
                            activation_focus.focus(window, cx);
                            super::super::link::activate(&href, window, cx);
                        }
                        // Let Base receive mouse-up and finish the selection gesture.
                    });
                let mut element = (link.inspect)(
                    element,
                    (first_offset == Some(source_offset)).then_some(&focus),
                );
                element.layout_as_root(
                    rectangle.size.map(gpui::AvailableSpace::Definite),
                    window,
                    cx,
                );
                element.prepaint_at(rectangle.origin, window, cx);
                links.push(element);
            }
        }
        Self {
            text,
            links,
            gesture: state.gesture.clone(),
        }
    }

    pub(super) fn paint(&mut self, window: &mut Window, cx: &mut App) {
        let gesture = self.gesture.clone();
        window.on_mouse_event(move |event: &gpui::MouseDownEvent, phase, _, _| {
            if phase == gpui::DispatchPhase::Capture && event.button == gpui::MouseButton::Left {
                gesture.set(Some((event.position, false)));
            }
        });
        let gesture = self.gesture.clone();
        window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, _, _| {
            if phase == gpui::DispatchPhase::Capture
                && let Some((start, moved)) = gesture.get()
            {
                gesture.set(Some((start, moved || dragged(start, event.position))));
            }
        });
        self.text.paint(window, cx);
        for link in &mut self.links {
            link.paint(window, cx);
        }
    }
}

fn link_rectangles(
    text: &str,
    range: Range<usize>,
    layout: &gpui::TextLayout,
) -> Vec<(usize, Bounds<Pixels>)> {
    let mut result: Vec<(usize, Bounds<Pixels>)> = Vec::new();
    for (offset, grapheme) in text[range.clone()].grapheme_indices(true) {
        let index = range.start + offset;
        if grapheme == "\n" {
            continue;
        }
        let hard_start = text[..index].rfind('\n').map_or(0, |newline| newline + 1);
        let local = index - hard_start;
        let Some(line) = layout.line_layout_for_index(index) else {
            continue;
        };
        let Some(origin) = layout.position_for_index(hard_start) else {
            continue;
        };
        // A wrap-boundary caret is left-biased in GPUI; the glyph at that
        // boundary belongs to the following row, so use its shaped run instead.
        let mut row = 0;
        let mut row_start = 0;
        for boundary in &line.wrap_boundaries {
            let offset =
                line.unwrapped_layout.runs[boundary.run_ix].glyphs[boundary.glyph_ix].index;
            if offset > local {
                break;
            }
            row += 1;
            row_start = offset;
        }
        let x = line.unwrapped_layout.x_for_index(local);
        let start = origin
            + gpui::point(
                x - line.unwrapped_layout.x_for_index(row_start),
                layout.line_height() * row,
            );
        let width = line.unwrapped_layout.x_for_index(local + grapheme.len()) - x;
        let rect = Bounds::new(start, gpui::size(width, layout.line_height()));
        if let Some((_, last)) = result.last_mut()
            && last.top() == rect.top()
            && last.right() == rect.left()
        {
            last.size.width += rect.size.width;
        } else {
            result.push((index, rect));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dragging_and_keyboard_activation_have_distinct_contracts() {
        let start = gpui::point(gpui::px(10.), gpui::px(10.));
        assert!(!dragged(start, start));
        assert!(dragged(start, gpui::point(gpui::px(20.), gpui::px(10.))));
        let mut event = gpui::KeyDownEvent {
            keystroke: gpui::Keystroke::parse("enter").unwrap(),
            is_held: false,
            prefer_character_input: false,
        };
        assert!(keyboard_activates(&event));
        event.is_held = true;
        assert!(!keyboard_activates(&event));
        event.is_held = false;
        event.keystroke = gpui::Keystroke::parse("space").unwrap();
        assert!(!keyboard_activates(&event));
        event.keystroke = gpui::Keystroke::parse("cmd-enter").unwrap();
        assert!(!keyboard_activates(&event));
    }
}
