//! Covers: merged literal/conditional hover styles and rerendered conditions.
//! Real: macro expansion, GPUI style refinement and pointer dispatch.
//! Doubles: GPUI TestPlatform window/text/GPU backend.
//! Requires: view-test-support; no native window or GPU.

use std::cell::Cell;
use std::rc::Rc;

use rake::gpui::{self, InputEvent, prelude::*};

use super::draw;

struct Host {
    selected: bool,
    branch: bool,
    color: Rc<Cell<gpui::Hsla>>,
}

impl Render for Host {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        let color = self.color.clone();

        if self.branch {
            return rake::view! {
                <div class={if self.selected {
                    "w-20 h-20 text-[#111111] hover:text-[#00ff00] hover:font-bold"
                } else {
                    "w-20 h-20 text-[#111111] hover:text-[#ff0000]"
                }}>
                    {gpui::canvas(move |_, window, _| color.set(window.text_style().color), |_, _, _, _| {}).size_full()}
                </div>
            };
        }

        rake::view! {
            <div class="w-20 h-20 text-[#111111] hover:text-[#ff0000] hover:font-bold"
                class:hover:text-[#00ff00]={self.selected}>
                {gpui::canvas(move |_, window, _| color.set(window.text_style().color), |_, _, _, _| {}).size_full()}
            </div>
        }
    }
}

#[test]
fn hover_classes_merge_once_and_conditional_colors_follow_rerenders() {
    for branch in [false, true] {
        let mut cx = gpui::TestAppContext::single();
        let color = Rc::new(Cell::new(gpui::Hsla::default()));

        let window = cx.add_window(|_, _| Host {
            selected: true,
            branch,
            color: color.clone(),
        });

        let move_to = |cx: &mut gpui::TestAppContext, coordinate: f32| {
            cx.update_window(window.into(), |_, window, cx| {
                window.dispatch_event(
                    gpui::MouseMoveEvent {
                        position: gpui::point(gpui::px(coordinate), gpui::px(coordinate)),
                        pressed_button: None,
                        modifiers: Default::default(),
                    }
                    .to_platform_input(),
                    cx,
                );
            })
            .unwrap();
            draw(cx, window);
        };

        move_to(&mut cx, 500.);
        assert_eq!(color.get(), gpui::rgb_to_hsla(gpui::rgb(0x111111)));
        move_to(&mut cx, 10.);
        assert_eq!(color.get(), gpui::rgb_to_hsla(gpui::rgb(0x00ff00)));

        window
            .update(&mut cx, |host, _, cx| {
                host.selected = false;
                cx.notify();
            })
            .unwrap();
        draw(&mut cx, window);

        assert_eq!(color.get(), gpui::rgb_to_hsla(gpui::rgb(0xff0000)));
    }
}
