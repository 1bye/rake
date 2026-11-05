//! Covers: recoloring an existing Base input through the Rake semantic palette.
//! Real: Base Input, GPUI rendering and the generated scene quads.
//! Doubles: GPUI TestPlatform. Requires: test-support, no display or GPU.

#![cfg(all(feature = "test-support", not(target_family = "wasm")))]

use gpui::{Context, Entity, Hsla, IntoElement, Render, TestAppContext, Window, prelude::*, rgb};

use gpui_base::{
    Theme,
    input::{Input, InputState},
};

use rake_ui::theme::{self, Semantic};

fn set_foreground(cx: &mut gpui::App, foreground: Hsla) {
    let mut semantic = *theme::current::<Semantic>();

    semantic.foreground = foreground;
    theme::install(cx, semantic);
}

struct Host(Entity<InputState>);

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::div().size_full().flex().child(Input::new(&self.0))
    }
}

#[test]
fn semantic_theme_repaints_the_existing_base_input_caret() {
    let mut cx = TestAppContext::single();
    let first = Hsla::from(rgb(0x12a67b));
    let second = Hsla::from(rgb(0xca1268));

    cx.update(|cx| {
        rake_ui::init(cx);
        set_foreground(cx, first);
        assert_eq!(Theme::global(cx).tokens.colors.foreground, first);
    });

    let window = cx.add_window(|window, cx| {
        Host(cx.new(|cx| {
            let mut state = InputState::new(window, cx);

            state.set_value("Theme probe", window, cx);
            state.focus(window, cx);

            state
        }))
    });

    cx.update_window(window.into(), |_, window, _| window.activate_window())
        .expect("activate test window");

    let draw = |cx: &mut TestAppContext| {
        cx.run_until_parked();

        cx.update_window(window.into(), |_, window, cx| {
            window.draw(cx).clear(cx);

            window.painted_quads()
        })
        .expect("draw Base input")
    };

    let initial = draw(&mut cx);

    assert!(
        initial.iter().any(|quad| quad.background == first.into()),
        "initial Base caret missing from frame: {initial:?}"
    );

    cx.update(|cx| set_foreground(cx, second));
    let updated = draw(&mut cx);

    assert!(
        updated.iter().any(|quad| quad.background == second.into()),
        "updated Base caret missing from frame: {updated:?}"
    );
    assert!(updated.iter().all(|quad| quad.background != first.into()));
}
