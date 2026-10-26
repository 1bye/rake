//! Covers: typed actions, key scopes, navigation, disabled activation and focus-visible styles.
//! Real: GPUI action dispatch, input events, focus, layout, and macro-generated controls.
//! Doubles: GPUI TestPlatform's window, text, and GPU backend.
//! Requires: the parent view-test-support target; no display or GPU.

use std::cell::Cell;
use std::rc::Rc;

use rake::gpui::{self, InputEvent as _, prelude::*};

use super::draw;

gpui::actions!(view_keyboard, [Save, Cancel, Local]);

struct KeyboardHost {
    root: gpui::FocusHandle,
    save: gpui::FocusHandle,
    middle: gpui::FocusHandle,
    last: gpui::FocusHandle,
    disabled: bool,
    key_context: &'static str,
    middle_index: isize,
    navigation: bool,
    consume_tab: bool,
    consume_default: bool,
    saved: usize,
    cancelled: usize,
    local: usize,
    parent_clicks: usize,
    focused_color: Rc<Cell<gpui::Hsla>>,
}

impl KeyboardHost {
    fn save(&mut self, _: &Save, _: &mut gpui::Window, cx: &mut Context<Self>) {
        if !self.disabled {
            self.saved += 1;
            cx.notify();
        }
    }

    fn cancel(&mut self, _: &Cancel, _: &mut gpui::Window, cx: &mut Context<Self>) {
        self.cancelled += 1;
        cx.notify();
    }
}

impl Render for KeyboardHost {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused_color = self.focused_color.clone();

        rake::view! {
            <section focus={&self.root} key-context={self.key_context} tab-navigation={self.navigation}
                style={|s| s.size_full()}
                on:Save={cx.listener(Self::save)}
                on:Cancel={cx.listener(Self::cancel)}
                onclick={cx.listener(|this, _, _, cx| { this.parent_clicks += 1; cx.notify(); })}
                onkeydown={cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                    if event.keystroke.key == "tab" {
                        if this.consume_tab { cx.stop_propagation(); }
                        if this.consume_default { window.prevent_default(); }
                    }
                })}>
                <button id="save" focus={&self.save} disabled={self.disabled}
                    style={|s| s.w(gpui::px(160.)).h(gpui::px(40.))
                        .text_color(gpui::rgb(0x111111))
                        .focus_visible(|s| s.text_color(gpui::rgb(0xff0000)))}
                    on:Local={cx.listener(|this, _, _, cx| { this.local += 1; cx.notify(); })}
                    onclick={|_, window, cx| window.dispatch_action(Box::new(Save), cx)}>
                    {gpui::canvas(move |_, window, _| focused_color.set(window.text_style().color), |_, _, _, _| {}).size_full()}
                </button>
                <button id="middle" tabindex={self.middle_index} focus={&self.middle}
                    style={|s| s.w(gpui::px(160.)).h(gpui::px(40.))}>"Middle"</button>
                <button id="last" focus={&self.last}
                    style={|s| s.w(gpui::px(160.)).h(gpui::px(40.))}
                    onclick={|_, window, cx| window.dispatch_action(Box::new(Cancel), cx)}>"Cancel"</button>
            </section>
        }
    }
}

fn setup() -> (gpui::TestAppContext, gpui::WindowHandle<KeyboardHost>) {
    let mut cx = gpui::TestAppContext::single();

    cx.update(|cx| {
        cx.bind_keys([
            gpui::KeyBinding::new("secondary-s", Save, Some("Editor")),
            gpui::KeyBinding::new("escape", Cancel, Some("Editor")),
            gpui::KeyBinding::new("l", Local, Some("Editor")),
        ])
    });
    let window = cx.add_window(|window, cx| {
        let root = cx.focus_handle();

        window.focus(&root, cx);

        KeyboardHost {
            root,
            save: cx.focus_handle(),
            middle: cx.focus_handle(),
            last: cx.focus_handle(),
            disabled: false,
            key_context: "Editor",
            middle_index: 0,
            navigation: true,
            consume_tab: false,
            consume_default: false,
            saved: 0,
            cancelled: 0,
            local: 0,
            parent_clicks: 0,
            focused_color: Rc::new(Cell::new(gpui::rgb_to_hsla(gpui::rgb(0x111111)))),
        }
    });

    draw(&mut cx, window);

    (cx, window)
}

fn keys(cx: &mut gpui::TestAppContext, window: gpui::WindowHandle<KeyboardHost>, keys: &str) {
    cx.update_window(window.into(), |_, window, cx| {
        let keystroke = gpui::Keystroke::parse(keys).expect("keystroke");

        window.dispatch_event(
            gpui::KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(gpui::KeyUpEvent { keystroke }.to_platform_input(), cx);
    })
    .expect("key down/up");
    draw(cx, window);
}

fn click(cx: &mut gpui::TestAppContext, window: gpui::WindowHandle<KeyboardHost>, y: f32) {
    cx.update_window(window.into(), |_, window, cx| {
        let position = gpui::point(gpui::px(20.), gpui::px(y));

        window.dispatch_event(
            gpui::MouseMoveEvent {
                position,
                pressed_button: None,
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(
            gpui::MouseDownEvent {
                position,
                button: gpui::MouseButton::Left,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(
            gpui::MouseUpEvent {
                position,
                button: gpui::MouseButton::Left,
                modifiers: Default::default(),
                click_count: 1,
            }
            .to_platform_input(),
            cx,
        );
    })
    .expect("mouse click");
    draw(cx, window);
}

#[test]
fn tab_shift_tab_and_negative_indices_follow_the_enabled_controls() {
    let (mut cx, window) = setup();

    keys(&mut cx, window, "tab");
    window
        .update(&mut cx, |host, window, _| {
            assert!(host.save.is_focused(window))
        })
        .unwrap();
    keys(&mut cx, window, "tab");
    window
        .update(&mut cx, |host, window, _| {
            assert!(host.middle.is_focused(window))
        })
        .unwrap();
    keys(&mut cx, window, "shift-tab");
    window
        .update(&mut cx, |host, window, _| {
            assert!(host.save.is_focused(window))
        })
        .unwrap();
    window
        .update(&mut cx, |host, _, cx| {
            host.middle_index = -1;
            cx.notify();
        })
        .unwrap();
    draw(&mut cx, window);
    keys(&mut cx, window, "tab");
    window
        .update(&mut cx, |host, window, _| {
            assert!(host.last.is_focused(window))
        })
        .unwrap();
    keys(&mut cx, window, "shift-tab");
    window
        .update(&mut cx, |host, window, _| {
            assert!(host.save.is_focused(window))
        })
        .unwrap();
}

#[test]
fn click_enter_space_and_shortcut_reach_the_same_typed_action_handler() {
    let (mut cx, window) = setup();

    click(&mut cx, window, 20.);
    window
        .update(&mut cx, |host, window, _| {
            assert!(host.save.is_focused(window), "click must focus Save");
            assert_eq!(host.saved, 1, "mouse click");
        })
        .unwrap();
    keys(&mut cx, window, "enter");
    window
        .read_with(&cx, |host, _| assert_eq!(host.saved, 2, "Enter"))
        .unwrap();
    keys(&mut cx, window, "space");
    window
        .read_with(&cx, |host, _| assert_eq!(host.saved, 3, "Space"))
        .unwrap();
    keys(&mut cx, window, "secondary-s");
    keys(&mut cx, window, "l");
    keys(&mut cx, window, "escape");
    window
        .read_with(&cx, |host, _| {
            assert_eq!(host.saved, 4);
            assert_eq!(host.local, 1);
            assert_eq!(host.cancelled, 1);
        })
        .unwrap();
}

#[test]
fn disabled_buttons_block_mouse_keyboard_and_local_actions_but_allow_navigation_out() {
    let (mut cx, window) = setup();

    keys(&mut cx, window, "tab");
    window
        .update(&mut cx, |host, _, cx| {
            host.disabled = true;
            cx.notify();
        })
        .unwrap();
    draw(&mut cx, window);
    keys(&mut cx, window, "enter");
    keys(&mut cx, window, "space");
    keys(&mut cx, window, "secondary-s");
    keys(&mut cx, window, "l");
    keys(&mut cx, window, "tab");
    window
        .update(&mut cx, |host, window, _| {
            assert!(host.middle.is_focused(window))
        })
        .unwrap();
    click(&mut cx, window, 20.);
    window
        .update(&mut cx, |host, window, _| {
            assert!(
                host.middle.is_focused(window),
                "disabled click must not take focus"
            );
            assert_eq!(host.saved, 0);
            assert_eq!(host.local, 0);
            assert_eq!(
                host.parent_clicks, 0,
                "disabled click must not activate an ancestor"
            );
        })
        .unwrap();
    keys(&mut cx, window, "shift-tab");
    window
        .update(&mut cx, |host, window, _| {
            assert!(host.last.is_focused(window))
        })
        .unwrap();
}

#[test]
fn tab_navigation_respects_consumed_events_disabled_navigation_and_modifiers() {
    let (mut cx, window) = setup();

    keys(&mut cx, window, "tab");

    for (consume_tab, consume_default, navigation, key) in [
        (true, false, true, "tab"),
        (false, true, true, "tab"),
        (false, false, false, "tab"),
        (false, false, true, "ctrl-tab"),
    ] {
        window
            .update(&mut cx, |host, _, cx| {
                host.consume_tab = consume_tab;
                host.consume_default = consume_default;
                host.navigation = navigation;
                cx.notify();
            })
            .unwrap();
        draw(&mut cx, window);
        keys(&mut cx, window, key);
        window
            .update(&mut cx, |host, window, _| {
                assert!(host.save.is_focused(window))
            })
            .unwrap();
    }
}

#[test]
fn focus_visible_styles_follow_keyboard_navigation_without_mouse_focus_styling() {
    let (mut cx, window) = setup();

    click(&mut cx, window, 20.);
    window
        .read_with(&cx, |host, _| {
            assert_eq!(
                host.focused_color.get(),
                gpui::rgb_to_hsla(gpui::rgb(0x111111))
            )
        })
        .unwrap();
    keys(&mut cx, window, "tab");
    keys(&mut cx, window, "shift-tab");
    window
        .read_with(&cx, |host, _| {
            assert_eq!(
                host.focused_color.get(),
                gpui::rgb_to_hsla(gpui::rgb(0xff0000))
            )
        })
        .unwrap();
}

#[test]
fn typed_shortcuts_require_the_matching_key_context() {
    let (mut cx, window) = setup();

    window
        .update(&mut cx, |host, _, cx| {
            host.key_context = "Other";
            cx.notify();
        })
        .unwrap();
    draw(&mut cx, window);
    keys(&mut cx, window, "secondary-s");
    window
        .read_with(&cx, |host, _| assert_eq!(host.saved, 0))
        .unwrap();
}

#[test]
fn disabling_between_key_down_and_up_cancels_activation() {
    let (mut cx, window) = setup();

    keys(&mut cx, window, "tab");
    cx.update_window(window.into(), |_, window, cx| {
        window.dispatch_event(
            gpui::KeyDownEvent {
                keystroke: gpui::Keystroke::parse("space").unwrap(),
                is_held: false,
                prefer_character_input: false,
            }
            .to_platform_input(),
            cx,
        );
    })
    .unwrap();
    window
        .update(&mut cx, |host, _, cx| {
            host.disabled = true;
            cx.notify();
        })
        .unwrap();
    draw(&mut cx, window);
    cx.update_window(window.into(), |_, window, cx| {
        window.dispatch_event(
            gpui::KeyUpEvent {
                keystroke: gpui::Keystroke::parse("space").unwrap(),
            }
            .to_platform_input(),
            cx,
        );
    })
    .unwrap();
    draw(&mut cx, window);
    window
        .read_with(&cx, |host, _| assert_eq!(host.saved, 0))
        .unwrap();
}
