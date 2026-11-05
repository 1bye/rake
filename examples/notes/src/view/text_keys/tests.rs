//! Covers Unicode editing and the platform text range bridge used by Fieldnotes.

use super::bind;
use crate::model::MAX_TITLE_BYTES;
use crate::view::fill_input;
use gpui_base::input::InputEditsEvent;
use rake::gpui::{self, AppContext, EntityInputHandler, Render, TestAppContext};
use rake::ui::input::{Input, InputState, Textarea, TextareaState};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

struct Body(gpui::Entity<TextareaState>);

impl Render for Body {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        fill_input(Textarea::new(&self.0))
    }
}

struct Field(gpui::Entity<InputState>);

impl Render for Field {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        fill_input(Input::new(&self.0))
    }
}

#[test]
fn body_deletes_a_grapheme_and_undo_restores_it() {
    const ORIGINAL: &str = "a👩🏽‍💻e\u{301}";

    let mut app = TestAppContext::single();

    app.update(|cx| {
        gpui_base::init(cx);
        bind(cx);
    });
    let (host, cx) = app.add_window_view(|window, cx| {
        Body(cx.new(|cx| {
            let mut state = TextareaState::new(window, cx)
                .grapheme_editing()
                .default_value(ORIGINAL);
            let end = ORIGINAL.encode_utf16().count();

            state.set_selected_text_range(end..end, window, cx);
            state.focus(window, cx);

            state
        }))
    });
    let body = host.read_with(cx, |host, _| host.0.clone());

    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.simulate_keystrokes("backspace");
    assert_eq!(body.read_with(cx, |body, _| body.value()), "a👩🏽‍💻");

    let command = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };

    cx.simulate_keystrokes(&format!("{command}-z"));
    assert_eq!(body.read_with(cx, |body, _| body.value()), ORIGINAL);

    let redo = if cfg!(target_os = "macos") {
        "cmd-shift-z"
    } else {
        "ctrl-y"
    };

    cx.simulate_keystrokes(redo);
    assert_eq!(body.read_with(cx, |body, _| body.value()), "a👩🏽‍💻");

    cx.update(|window, cx| {
        body.update(cx, |body, cx| {
            body.set_selected_text_range(1..1, window, cx)
        });
    });
    cx.simulate_keystrokes("delete");
    assert_eq!(body.read_with(cx, |body, _| body.value()), "a");
}

#[test]
fn arrows_move_and_select_whole_graphemes() {
    let mut app = TestAppContext::single();

    app.update(|cx| {
        gpui_base::init(cx);
        bind(cx);
    });
    let (host, cx) = app.add_window_view(|window, cx| {
        Body(cx.new(|cx| {
            let mut state = TextareaState::new(window, cx)
                .grapheme_editing()
                .default_value("ae\u{301}");

            state.set_selected_text_range(3..3, window, cx);
            state.focus(window, cx);

            state
        }))
    });
    let body = host.read_with(cx, |host, _| host.0.clone());

    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.simulate_keystrokes("shift-left");
    assert_eq!(
        body.read_with(cx, |body, _| body.selected_value()),
        "e\u{301}"
    );

    cx.simulate_keystrokes("shift-right");
    assert_eq!(body.read_with(cx, |body, _| body.selected_range()), 4..4);

    cx.simulate_keystrokes("left backspace");
    assert_eq!(body.read_with(cx, |body, _| body.value()), "e\u{301}");
}

#[test]
fn arrows_keep_both_carets_on_grapheme_boundaries() {
    let mut app = TestAppContext::single();
    app.update(gpui_base::init);
    let (host, cx) = app.add_window_view(|window, cx| {
        Body(cx.new(|cx| {
            let mut state = TextareaState::new(window, cx)
                .grapheme_editing()
                .default_value("ab\nae\u{301}");

            state.set_selected_range(7..7, cx);
            state.add_selection(2..2, cx);
            state.focus(window, cx);

            state
        }))
    });
    let body = host.read_with(cx, |host, _| host.0.clone());

    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.simulate_keystrokes("left x");
    assert_eq!(
        body.read_with(cx, |body, _| body.value()),
        "axb\naxe\u{301}"
    );
}

#[test]
fn body_platform_ranges_stay_on_unicode_boundaries() {
    let mut app = TestAppContext::single();

    app.update(gpui_base::init);
    let (body, cx) = app.add_window_view(|window, cx| {
        TextareaState::new(window, cx)
            .grapheme_editing()
            .default_value("a😀b")
    });

    cx.update(|window, cx| {
        body.update(cx, |body, cx| {
            let mut adjusted = None;

            assert_eq!(
                body.text_for_range(1..3, &mut adjusted, window, cx)
                    .as_deref(),
                Some("😀")
            );
            assert_eq!(adjusted, Some(1..3));

            body.set_selected_text_range(2..999, window, cx);
            let selection = body
                .selected_text_range(false, window, cx)
                .expect("body selection");

            assert_eq!(selection.range, 1..4);
        });
    });
}

#[test]
fn single_line_input_deletes_a_whole_grapheme() {
    let mut app = TestAppContext::single();

    app.update(|cx| {
        gpui_base::init(cx);
        bind(cx);
    });
    let (host, cx) = app.add_window_view(|window, cx| {
        Field(cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .grapheme_editing()
                .default_value("e\u{301}");

            state.set_selected_text_range(2..2, window, cx);
            state.focus(window, cx);

            state
        }))
    });
    let field = host.read_with(cx, |host, _| host.0.clone());

    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.simulate_keystrokes("backspace");
    assert_eq!(field.read_with(cx, |field, _| field.value()), "");
}

#[test]
fn rejected_title_composition_restores_text_selection_and_history() {
    let mut app = TestAppContext::single();
    let original = "a".repeat(MAX_TITLE_BYTES - 3);
    let prior = format!("{original}b");
    let later = format!("{prior}c");
    let selection = original.len()..prior.len();
    let command = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    let redo = if cfg!(target_os = "macos") {
        "cmd-shift-z"
    } else {
        "ctrl-y"
    };

    app.update(gpui_base::init);
    let (host, cx) = app.add_window_view(|window, cx| {
        Field(cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .default_value(original.clone())
                .validate(|text, _| text.len() <= MAX_TITLE_BYTES);

            state.set_selected_range(original.len()..original.len(), cx);
            state.focus(window, cx);

            state
        }))
    });
    let field = host.read_with(cx, |host, _| host.0.clone());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.simulate_keystrokes("b left right c");
    cx.simulate_keystrokes(&format!("{command}-z"));
    assert_eq!(field.read_with(cx, |state, _| state.value()), prior);

    cx.update(|window, cx| {
        field.update(cx, |state, cx| {
            state.set_selected_range_with_direction(selection.clone(), true, cx);
            state.replace_and_mark_text_in_range(None, "xy", None, window, cx);
            state.replace_and_mark_text_in_range(None, "abc", None, window, cx);
            state.replace_text_in_range(None, "漢字", window, cx);

            assert_eq!(state.value(), prior);
            assert_eq!(state.selected_range(), selection);
            assert!(state.is_selection_reversed());
            assert!(state.marked_text_range(window, cx).is_none());
            state.unmark_text(window, cx);
        });
    });

    cx.simulate_keystrokes(redo);
    assert_eq!(field.read_with(cx, |state, _| state.value()), later);
    cx.simulate_keystrokes(&format!("{command}-z {command}-z"));
    assert_eq!(field.read_with(cx, |state, _| state.value()), original);
}

#[test]
fn rejected_second_composition_preserves_the_subscriber_text_baseline() {
    let mut app = TestAppContext::single();

    app.update(gpui_base::init);
    let (field, cx) = app.add_window_view(|window, cx| {
        InputState::new(window, cx)
            .default_value("ab")
            .validate(|text, _| text.is_ascii())
    });
    let observed = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&observed);
    let _subscription = cx.update(|_, cx| {
        cx.subscribe(&field, move |_, event: &InputEditsEvent, _| {
            seen.borrow_mut().push(event.clone());
        })
    });

    cx.update(|window, cx| {
        field.update(cx, |state, cx| {
            state.set_selected_range(2..2, cx);
            state.replace_and_mark_text_in_range(None, "x", None, window, cx);
            state.unmark_text(window, cx);
            state.replace_and_mark_text_in_range(None, "y", None, window, cx);
            state.replace_text_in_range(None, "漢", window, cx);
            state.unmark_text(window, cx);
        });
    });
    assert!(observed.borrow().is_empty());
    assert_eq!(field.read_with(cx, |state, _| state.value()), "abx");

    cx.update(|window, cx| {
        field.update(cx, |state, cx| {
            state.replace_text_in_range(None, "z", window, cx);
        });
    });

    let mut subscriber_text = String::from("ab");
    let events = observed.borrow();

    assert_eq!(events.len(), 1);
    for (edit, after) in events[0].edits() {
        let range = edit.start_byte..edit.old_end_byte;
        let replacement = after.slice(edit.start_byte..edit.new_end_byte).to_string();

        assert!(
            subscriber_text.get(range.clone()).is_some(),
            "edit {range:?} must apply to the last published text {subscriber_text:?}"
        );
        subscriber_text.replace_range(range, &replacement);
        assert_eq!(subscriber_text, after.to_string());
    }

    assert_eq!(subscriber_text, "abxz");
}

#[test]
fn rejected_body_edits_preserve_history_and_cancel_preedit() {
    let mut app = TestAppContext::single();
    let rejections = Rc::new(Cell::new(0));
    let rejected = Rc::clone(&rejections);
    let command = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };

    app.update(gpui_base::init);
    let (host, cx) = app.add_window_view(|window, cx| {
        Body(cx.new(|cx| {
            let mut state = TextareaState::new(window, cx)
                .max_bytes(4, move |_| rejected.set(rejected.get() + 1))
                .default_value("ab");

            state.set_selected_range(2..2, cx);
            state.focus(window, cx);

            state
        }))
    });
    let body = host.read_with(cx, |host, _| host.0.clone());

    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.simulate_keystrokes("c");
    cx.update(|window, cx| {
        body.update(cx, |state, cx| {
            state.replace_text_in_range(Some(3..3), "é", window, cx);
            assert_eq!(state.value(), "abc");

            state.replace_and_mark_text_in_range(None, "x", None, window, cx);
            assert_eq!(state.value(), "abcx");
            state.replace_text_in_range(None, "漢", window, cx);
            assert_eq!(state.value(), "abc");
            assert!(state.marked_text_range(window, cx).is_none());

            state.add_selection(0..0, cx);
            state.replace_text_in_range(None, "z", window, cx);
            assert_eq!(state.value(), "abc");
        });
    });
    assert_eq!(rejections.get(), 3);
    cx.simulate_keystrokes(&format!("{command}-z"));
    assert_eq!(body.read_with(cx, |state, _| state.value()), "ab");
}

#[test]
fn title_keeps_word_boundaries_in_inserted_and_pasted_text() {
    let mut app = TestAppContext::single();
    let command = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };

    app.update(gpui_base::init);
    let (host, cx) = app.add_window_view(|window, cx| {
        Field(cx.new(|cx| {
            let state = InputState::new(window, cx).line_breaks_as_spaces();

            state.focus(window, cx);

            state
        }))
    });
    let title = host.read_with(cx, |host, _| host.0.clone());

    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        title.update(cx, |state, cx| {
            state.replace_text_in_range(None, "First\nSecond", window, cx);
        });
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("Third\rFourth".into()));
    });
    assert_eq!(
        title.read_with(cx, |state, _| state.value()),
        "First Second"
    );

    cx.simulate_keystrokes(&format!("{command}-a {command}-v"));
    assert_eq!(
        title.read_with(cx, |state, _| state.value()),
        "Third Fourth"
    );
}
