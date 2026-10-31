//! Covers: Unicode run ranges, nested style inheritance, coalescing, and accessible values.
//! Real: paragraph composition and GPUI styled-text construction.
//! Doubles: none; frame checks below use GPUI TestPlatform when enabled.
//! Requires: experimental-view; frame checks also require native view-test-support.

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
