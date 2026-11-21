//! Covers: Unicode run ranges, nested style inheritance, coalescing, and accessible values.
//! Real: paragraph composition and GPUI styled-text construction.
//! Doubles: none; frame checks below use GPUI TestPlatform when enabled.
//! Requires: gpui; frame checks also require native view-test-support.

use super::*;

#[test]
fn unicode_empty_segments_and_adjacent_styles_keep_valid_byte_ranges() {
    let mut content = ParagraphContent::default();
    let plain = RunStyle::default();
    let bold = plain.clone().highlight(gpui::FontWeight::BOLD.into());

    content.push("é", &plain);
    content.push("", &bold);
    content.push("🙂", &plain);
    content.push("你好", &bold);

    assert_eq!(content.text, "é🙂你好");
    assert_eq!(content.runs.len(), 2);
    assert_eq!(content.runs[0].range, 0.."é🙂".len());
    assert_eq!(content.runs[1].range, "é🙂".len().."é🙂你好".len());
    assert_eq!(
        content.runs[1].style.highlight.font_weight,
        Some(gpui::FontWeight::BOLD)
    );

    let text = content.finish();
    let mut node = accesskit::Node::new(text.a11y_role().unwrap());

    text.write_a11y_info(&mut node);

    assert_eq!(node.value(), Some("é🙂你好"));
}

#[test]
fn nested_highlights_and_families_override_without_mutating_the_parent_style() {
    let parent = RunStyle::default()
        .highlight(gpui::FontWeight::BOLD.into())
        .font_family("Parent");

    let nested = parent
        .clone()
        .highlight(gpui::FontStyle::Italic.into())
        .font_family("Child");

    assert_eq!(nested.highlight.font_weight, Some(gpui::FontWeight::BOLD));
    assert_eq!(nested.highlight.font_style, Some(gpui::FontStyle::Italic));
    assert_eq!(nested.family.as_deref(), Some("Child"));
    assert_eq!(parent.highlight.font_style, None);
    assert_eq!(parent.family.as_deref(), Some("Parent"));

    let reset = nested.highlight(gpui::FontWeight::NORMAL.into());

    assert_eq!(reset.highlight.font_weight, Some(gpui::FontWeight::NORMAL));
    assert_eq!(reset.highlight.font_style, Some(gpui::FontStyle::Italic));
}

#[test]
fn empty_content_constructs_an_empty_accessible_text_element() {
    let text = ParagraphContent::default().finish();
    let mut node = accesskit::Node::new(text.a11y_role().unwrap());

    text.write_a11y_info(&mut node);

    assert_eq!(node.value(), Some(""));
}

#[cfg(all(feature = "test-support", not(target_family = "wasm")))]
mod frames {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui::prelude::*;
    use unicode_segmentation::UnicodeSegmentation as _;

    use super::*;

    const SOURCE: &str = "one two three four five six seven eight nine ten eleven twelve";

    struct Host {
        wrap: bool,
        ellipsis: bool,
        max_lines: Option<usize>,
        layout: Rc<RefCell<layout::ParagraphLayout>>,
        source: &'static str,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
            let mut content = ParagraphContent::default();

            content.push("Count ", &RunStyle::default());
            content.push(
                self.source,
                &RunStyle::default().highlight(gpui::FontWeight::BOLD.into()),
            );
            let mut text = content.finish();

            text.layout = self.layout.clone();
            let mut node = accesskit::Node::new(text.a11y_role().unwrap());

            text.write_a11y_info(&mut node);

            assert_eq!(
                node.value(),
                Some(format!("Count {}", self.source).as_str())
            );

            gpui::div()
                .w(gpui::px(80.))
                .text_size(gpui::px(14.))
                .line_height(gpui::px(20.))
                .when_else(
                    self.wrap,
                    |s| s.whitespace_normal(),
                    |s| s.whitespace_nowrap(),
                )
                .when(self.ellipsis, |s| s.text_ellipsis())
                .when_some(self.max_lines, |s, lines| s.line_clamp(lines))
                .child(text)
        }
    }

    #[test]
    fn gpui_shapes_wrapped_and_truncated_runs_without_truncating_accessible_content() {
        for (wrap, ellipsis, max_lines, source) in [
            (true, false, None, SOURCE),
            (false, true, None, SOURCE),
            (true, true, Some(2), SOURCE),
            (true, true, Some(1), "é🙂\n你好\nlast"),
            (true, false, Some(2), SOURCE),
            (false, true, None, "é 👩‍💻 café 🇺🇦 你好 é 👩‍💻 café 🇺🇦 你好"),
        ] {
            let mut cx = gpui::TestAppContext::single();
            let layout = Rc::new(RefCell::new(layout::ParagraphLayout::default()));

            let window = cx.add_window(|_, _| Host {
                wrap,
                ellipsis,
                max_lines,
                layout: layout.clone(),
                source,
            });

            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
                .unwrap();
            let layout = layout.borrow();

            let lines = layout
                .lines
                .iter()
                .map(|line| line.wrap_boundaries.len() + 1)
                .sum::<usize>();

            let shown = layout
                .lines
                .iter()
                .map(|line| line.text.as_ref())
                .collect::<Vec<_>>()
                .join("\n");

            if ellipsis {
                assert!(shown.ends_with('…'), "{shown}");
                assert!(lines <= max_lines.unwrap_or(1), "{shown}");
                let prefix = shown.strip_suffix('…').unwrap();
                let original = format!("Count {source}");

                assert!(original.starts_with(prefix));
                assert!(
                    original
                        .grapheme_indices(true)
                        .any(|(index, _)| index == prefix.len())
                );
            } else if let Some(limit) = max_lines {
                assert!(lines <= limit);
                assert!(!shown.ends_with('…'));
            } else {
                assert!(lines > 2);
                assert_eq!(shown, format!("Count {source}"));
            }
        }
    }
}

#[cfg(all(feature = "test-support", not(target_family = "wasm")))]
mod selection_frames {
    use super::*;
    use gpui::{Context, Modifiers, MouseButton, Render, TestAppContext, prelude::*};

    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    struct Host {
        visits: Rc<Cell<usize>>,
        link_text: &'static str,
        width: f32,
        max_lines: Option<usize>,
        measured: Rc<RefCell<layout::ParagraphLayout>>,
    }

    impl Render for Host {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            crate::init(cx);
            let visits = self.visits.clone();

            crate::view::install_link_router(window, cx, move |_, _| visits.set(visits.get() + 1));
            let mut content = ParagraphContent::default();

            content.push(" alpha ", &RunStyle::default());
            let start = content.text_len();

            content.push(
                self.link_text,
                &RunStyle::default().highlight(gpui::FontWeight::BOLD.into()),
            );
            content.link(start, "/next", "next", None, |link, _| {
                link.into_any_element()
            });
            content.push(" omega ", &RunStyle::default());
            let mut text = content.finish().select_text(true);

            text.layout = self.measured.clone();

            gpui::div().size_full().child(
                gpui::div()
                    .w(gpui::px(self.width))
                    .whitespace_normal()
                    .when_some(self.max_lines, |node, lines| {
                        node.text_ellipsis().line_clamp(lines)
                    })
                    .child(text),
            )
        }
    }

    #[test]
    fn plain_click_follows_link_but_drag_selects_visible_runs() {
        let mut cx = TestAppContext::single();
        let visits = Rc::new(Cell::new(0));
        let measured = Rc::new(RefCell::new(layout::ParagraphLayout::default()));

        let (_, cx) = cx.add_window_view(|window, cx| {
            crate::init(cx);
            let view = cx.new(|_| Host {
                visits: visits.clone(),
                link_text: "link",
                width: 240.,
                max_lines: None,
                measured: measured.clone(),
            });

            gpui_base::Root::new(view, window, cx)
        });

        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        let position = {
            let measured = measured.borrow();

            measured.lines[0]
                .position_for_index(8, measured.line_height)
                .unwrap()
                + gpui::point(gpui::px(1.), measured.line_height / 2.)
        };

        cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(position, MouseButton::Left, Modifiers::default());
        assert_eq!(visits.get(), 1);
        cx.simulate_keystrokes("enter");
        cx.simulate_event(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse("enter").unwrap(),
        });
        assert_eq!(visits.get(), 2);
        cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(
            position + gpui::point(gpui::px(5.), gpui::px(0.)),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        cx.simulate_mouse_up(
            position + gpui::point(gpui::px(5.), gpui::px(0.)),
            MouseButton::Left,
            Modifiers::default(),
        );
        assert_eq!(visits.get(), 2);
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        let start = gpui::point(gpui::px(1.), position.y);

        cx.simulate_mouse_move(start, None, Modifiers::default());
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(
            gpui::point(gpui::px(230.), position.y),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        cx.simulate_mouse_up(
            gpui::point(gpui::px(230.), position.y),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            assert_eq!(
                gpui_base::TextSelection::selected_text(window, cx),
                " alpha link omega "
            );
        });
        cx.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-c"
        } else {
            "ctrl-c"
        });
        cx.update(|_, cx| {
            assert_eq!(
                cx.read_from_clipboard().and_then(|item| item.text()),
                Some(" alpha link omega ".into())
            )
        });
    }

    #[test]
    fn wrapped_link_activates_on_its_second_text_row() {
        let mut cx = TestAppContext::single();
        let visits = Rc::new(Cell::new(0));
        let measured = Rc::new(RefCell::new(layout::ParagraphLayout::default()));

        let (_, cx) = cx.add_window_view(|window, cx| {
            crate::init(cx);
            let view = cx.new(|_| Host {
                visits: visits.clone(),
                measured: measured.clone(),
                link_text: "one two three four five six seven",
                width: 120.,
                max_lines: None,
            });

            gpui_base::Root::new(view, window, cx)
        });

        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        let position = {
            let measured = measured.borrow();
            let line = &measured.lines[0];

            assert!(line.wrap_boundaries.len() >= 2);
            let boundary = &line.wrap_boundaries[0];
            let index = line.unwrapped_layout.runs[boundary.run_ix].glyphs[boundary.glyph_ix].index;

            line.position_for_index(index + 1, measured.line_height)
                .unwrap()
                + gpui::point(gpui::px(1.), measured.line_height / 2.)
        };

        cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(position, MouseButton::Left, Modifiers::default());
        assert_eq!(visits.get(), 1);
        cx.simulate_keystrokes("enter");
        cx.simulate_event(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse("enter").unwrap(),
        });
        assert_eq!(visits.get(), 2);
        let blank = gpui::point(gpui::px(230.), position.y);

        cx.simulate_mouse_down(blank, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(blank, MouseButton::Left, Modifiers::default());
        assert_eq!(visits.get(), 2);
    }

    #[test]
    fn truncated_selection_copies_only_painted_text() {
        let mut cx = TestAppContext::single();
        let measured = Rc::new(RefCell::new(layout::ParagraphLayout::default()));

        let (_, cx) = cx.add_window_view(|window, cx| {
            crate::init(cx);
            let view = cx.new(|_| Host {
                visits: Rc::new(Cell::new(0)),
                measured: measured.clone(),
                link_text: "one two three four five six seven",
                width: 120.,
                max_lines: Some(1),
            });

            gpui_base::Root::new(view, window, cx)
        });

        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        let visible = measured
            .borrow()
            .lines
            .iter()
            .map(|line| line.text.as_ref())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(visible.len() < " alpha one two three four five six seven omega ".len());
        let start = gpui::point(gpui::px(1.), measured.borrow().line_height / 2.);
        let end = gpui::point(gpui::px(230.), start.y);

        cx.simulate_mouse_move(start, None, Modifiers::default());
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::default());
        cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            assert_eq!(gpui_base::TextSelection::selected_text(window, cx), visible);
        });
    }
}

#[test]
fn links_split_identical_styles_and_reject_unsafe_hrefs() {
    let mut content = ParagraphContent::default();
    let style = RunStyle::default();

    content.push("one ", &style);
    content.push("two", &style);
    content.link(4, "/two", "two", None, |link, _| link.into_any_element());
    content.push(" three", &style);
    assert_eq!(
        content
            .runs
            .iter()
            .map(|run| run.range.clone())
            .collect::<Vec<_>>(),
        [0..4, 4..7, 7..13]
    );
    content.link(0, "javascript:alert(1)", "unsafe", None, |link, _| {
        link.into_any_element()
    });
    assert_eq!(content.links.len(), 1);
}
