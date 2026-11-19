//! Covers: the public motion facade, render-free frames, interruption and reduced motion.
//! Real: Rake's patched GPUI presentation layers. Doubles: GPUI TestPlatform.
//! Requires: motion and view-test-support; no native display or GPU.

#![cfg(all(
    feature = "motion",
    feature = "view-test-support",
    not(feature = "gpui-fast"),
    not(target_family = "wasm")
))]

use rake::gpui::{self, prelude::*};
use rake::motion::{self, Spring, Target};
use std::cell::Cell;
use std::rc::Rc;

const CARD: &str = "card";

struct Card {
    renders: Rc<Cell<usize>>,
}

impl Render for Card {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);

        motion::layer(CARD, gpui::div().size(gpui::px(40.)).bg(gpui::black()))
    }
}

fn mount(cx: &mut gpui::TestAppContext) -> (Rc<Cell<usize>>, &mut gpui::VisualTestContext) {
    let renders = Rc::new(Cell::new(0));
    let view_renders = renders.clone();
    let (_, cx) = cx.add_window_view(|_, _| Card {
        renders: view_renders,
    });

    cx.run_until_parked();
    (renders, cx)
}

fn right(distance: f32) -> Target {
    Target::offset(gpui::px(distance), gpui::px(0.))
}

#[gpui::test]
fn animation_frames_do_not_render_the_view(cx: &mut gpui::TestAppContext) {
    let (renders, cx) = mount(cx);
    let renders_before = renders.get();

    cx.update(|window, cx| motion::animate(CARD, right(120.), Spring::SMOOTH, window, cx));

    for _ in 0..4 {
        cx.simulate_frame_request(gpui::RequestFrameOptions::default());
    }

    assert_eq!(renders.get(), renders_before);
    assert!(cx.update(|window, _| motion::is_animating(CARD, window)));
}

#[gpui::test]
fn an_interrupted_layer_keeps_its_momentum(cx: &mut gpui::TestAppContext) {
    let (_, cx) = mount(cx);

    cx.update(|window, cx| {
        motion::animate(CARD, right(100.), Spring::SMOOTH, window, cx);
        window.advance_presentation_layers_for_test(0.05);

        let before = motion::value(CARD, window).x();

        motion::animate(CARD, Target::REST, Spring::SMOOTH, window, cx);
        window.advance_presentation_layers_for_test(0.001);

        assert!(motion::value(CARD, window).x() > before);
    });
}

#[gpui::test]
fn a_layer_settles_at_its_target(cx: &mut gpui::TestAppContext) {
    let (_, cx) = mount(cx);
    let target = right(64.).with_opacity(0.5);

    cx.update(|window, cx| {
        motion::animate(CARD, target, Spring::BOUNCY, window, cx);
        window.advance_presentation_layers_for_test(10.);

        assert!(!motion::is_animating(CARD, window));
        assert_eq!(motion::value(CARD, window), target);
    });
}

#[gpui::test]
fn jump_cancels_a_running_spring(cx: &mut gpui::TestAppContext) {
    let (_, cx) = mount(cx);
    let target = right(24.);

    cx.update(|window, cx| {
        motion::animate(CARD, right(200.), Spring::SNAPPY, window, cx);
        motion::jump(CARD, target, window);

        assert!(!motion::is_animating(CARD, window));
        assert_eq!(motion::value(CARD, window), target);
    });
}

#[gpui::test]
fn reduced_motion_moves_immediately(cx: &mut gpui::TestAppContext) {
    let (_, cx) = mount(cx);
    let target = right(32.);

    cx.update(|window, cx| {
        cx.set_reduce_motion(true);
        motion::animate(CARD, target, Spring::SMOOTH, window, cx);

        assert!(!motion::is_animating(CARD, window));
        assert_eq!(motion::value(CARD, window), target);
    });
}
