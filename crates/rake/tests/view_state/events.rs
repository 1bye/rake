//! Covers: canonical pointer/hover/wheel dispatch and disabled event suppression.
//! Real: GPUI hit testing, input dispatch, and macro-created listeners.
//! Doubles: GPUI TestPlatform's window/text/GPU backend.
//! Requires: native view-test-support; no display or GPU.

use rake::gpui::{self, InputEvent as _, prelude::*};

use super::draw;

struct Host {
    disabled: bool,
    events: Vec<&'static str>,
}

impl Render for Host {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        rake::view! {
            <button disabled={self.disabled} style={|s| s.w(gpui::px(200.)).h(gpui::px(80.))}
                on:right-mouse-down={cx.listener(|this, _, _, _| this.events.push("right-down"))}
                on:right-mouse-up={cx.listener(|this, _, _, _| this.events.push("right-up"))}
                on:middle-mouse-down={cx.listener(|this, _, _, _| this.events.push("middle-down"))}
                on:middle-mouse-up={cx.listener(|this, _, _, _| this.events.push("middle-up"))}
                on:hover={cx.listener(|this, hovered: &bool, _, _| this.events.push(if *hovered { "enter" } else { "leave" }))}
                on:scroll-wheel={cx.listener(|this, _, _, _| this.events.push("wheel"))}>
                "Events"
            </button>
        }
    }
}

fn pointer_events(cx: &mut gpui::TestAppContext, handle: gpui::WindowHandle<Host>) {
    cx.update_window(handle.into(), |_, window, cx| {
        let inside = gpui::point(gpui::px(20.), gpui::px(20.));
        let outside = gpui::point(gpui::px(300.), gpui::px(300.));

        for position in [outside, inside] {
            window.dispatch_event(
                gpui::MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
        }

        for button in [gpui::MouseButton::Right, gpui::MouseButton::Middle] {
            window.dispatch_event(
                gpui::MouseDownEvent {
                    position: inside,
                    button,
                    modifiers: Default::default(),
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                gpui::MouseUpEvent {
                    position: inside,
                    button,
                    modifiers: Default::default(),
                    click_count: 1,
                }
                .to_platform_input(),
                cx,
            );
        }

        window.dispatch_event(
            gpui::ScrollWheelEvent {
                position: inside,
                delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(-20.))),
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(
            gpui::MouseMoveEvent {
                position: outside,
                pressed_button: None,
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
}

#[test]
fn canonical_pointer_events_dispatch_and_disabled_buttons_suppress_them() {
    let mut cx = gpui::TestAppContext::single();

    let window = cx.add_window(|_, _| Host {
        disabled: false,
        events: Vec::new(),
    });

    draw(&mut cx, window);
    pointer_events(&mut cx, window);
    window
        .read_with(&cx, |host, _| {
            for expected in [
                "right-down",
                "right-up",
                "middle-down",
                "middle-up",
                "enter",
                "leave",
                "wheel",
            ] {
                assert!(
                    host.events.contains(&expected),
                    "missing {expected}: {:?}",
                    host.events
                );
            }
        })
        .unwrap();
    window
        .update(&mut cx, |host, _, cx| {
            host.disabled = true;
            cx.notify();
        })
        .unwrap();
    draw(&mut cx, window);
    window
        .update(&mut cx, |host, _, _| host.events.clear())
        .unwrap();
    pointer_events(&mut cx, window);
    window
        .read_with(&cx, |host, _| {
            assert!(host.events.is_empty(), "disabled events: {:?}", host.events)
        })
        .unwrap();
}
