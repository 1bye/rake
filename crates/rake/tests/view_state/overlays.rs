//! Covers: overlay ordering, Tab containment, focus restoration, anchors and pointer isolation.
//! Real: public overlay controls, view! components, GPUI input/focus/layout and action dispatch.
//! Doubles: GPUI TestPlatform's window/text/GPU backend.
//! Requires: native view-test-support; no display or GPU.

use rake::gpui::{self, InputEvent as _, prelude::*};

use rake::overlay::{
    AnchorState, Dialog, OverlayAnchor, OverlayHost, OverlayHostState, OverlayState, Popover,
};

use super::draw as draw_frame;

// The host assigns initial focus after mount; paint that requested second frame
// so GPUI installs focused controls' Enter/Space listeners before sending input.
fn draw(cx: &mut gpui::TestAppContext, window: gpui::WindowHandle<Host>) {
    draw_frame(cx, window);
    cx.run_until_parked();
    draw_frame(cx, window);
    cx.run_until_parked();
}

gpui::actions!(overlay_tests, [BackgroundAction]);

struct Host {
    overlays: OverlayHostState,
    anchor: AnchorState,
    popup: OverlayState,
    dialog: OverlayState,
    body: gpui::FocusHandle,
    trigger: gpui::FocusHandle,
    other: gpui::FocusHandle,
    popup_first: gpui::FocusHandle,
    popup_last: gpui::FocusHandle,
    dialog_first: gpui::FocusHandle,
    dialog_last: gpui::FocusHandle,
    disabled: gpui::FocusHandle,
    scroll: gpui::ScrollHandle,
    empty_dialog: bool,
    show_anchor: bool,
    show_popup: bool,
    show_dialog: bool,
    consume_tab: bool,
    consume_escape: bool,
    anchor_x: f32,
    anchor_y: f32,
    clicks: usize,
    actions: usize,
}

impl Render for Host {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let body = rake::view! {
            <section focus={&self.body} scroll={&self.scroll} key-context="Background" style={|s| s.size_full().overflow_y_scroll()}
                on:BackgroundAction={cx.listener(|this, _, _, cx| { this.actions += 1; cx.notify(); })}
                onclick={cx.listener(|this, _, _, cx| { this.clicks += 1; cx.notify(); })}>
                <div style={|s| s.absolute().left(gpui::px(self.anchor_x)).top(gpui::px(self.anchor_y))}>
                    if self.show_anchor {
                        <OverlayAnchor state={self.anchor.clone()}>
                            <button focus={&self.trigger} style={|s| s.w(gpui::px(120.)).h(gpui::px(32.))}
                                onclick={cx.listener(|this, _, window, cx| { this.popup.open(window, cx); })}>"Actions"</button>
                        </OverlayAnchor>
                    }
                </div>
                <button focus={&self.other} style={|s| s.w(gpui::px(100.)).h(gpui::px(32.))}>"Background"</button>
                <div style={|s| s.h(gpui::px(2000.)).flex_shrink_0()}/>
            </section>
        }.into_any_element();

        let footer = rake::view! {
            <button focus={&self.dialog_last} style={|s| s.h(gpui::px(32.))}
                onkeydown={cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                    if this.consume_tab && event.keystroke.key == "tab" {
                        window.focus_next(cx);
                        cx.stop_propagation();
                    }
                })}
                onclick={cx.listener(|this, _, window, cx| { this.dialog.close(window, cx); })}>"Close dialog"</button>
        }.into_any_element();

        rake::view! {
            <OverlayHost state={self.overlays.clone()} body={body}>
                if self.show_popup {
                    <Popover state={self.popup.clone()} anchor={self.anchor.clone()}>
                        <button focus={&self.popup_first} style={|s| s.h(gpui::px(32.))}
                            onclick={cx.listener(|this, _, window, cx| { this.dialog.open(window, cx); })}>"Open dialog"</button>
                        <button focus={&self.popup_last} style={|s| s.h(gpui::px(32.))}
                            onclick={cx.listener(|this, _, window, cx| { this.popup.close(window, cx); })}>"Close popup"</button>
                    </Popover>
                }
                if self.show_dialog {
                    <Dialog state={self.dialog.clone()} title={"Dialog".into()} footer={if self.empty_dialog { None } else { Some(footer) }}>
                        if !self.empty_dialog {
                        <button focus={&self.dialog_first} style={|s| s.h(gpui::px(32.))}
                            onkeydown={cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                                if this.consume_escape && event.keystroke.key == "escape" {
                                    window.prevent_default();
                                    cx.stop_propagation();
                                }
                            })}>"First"</button>
                        <button disabled={true} focus={&self.disabled} style={|s| s.h(gpui::px(32.))}>"Disabled"</button>
                        }
                    </Dialog>
                }
            </OverlayHost>
        }
    }
}

fn setup() -> (gpui::TestAppContext, gpui::WindowHandle<Host>) {
    let mut cx = gpui::TestAppContext::single();

    cx.update(|cx| {
        cx.bind_keys([gpui::KeyBinding::new(
            "b",
            BackgroundAction,
            Some("Background"),
        )])
    });
    let window = cx.add_window(|window, cx| {
        let overlays = OverlayHostState::new(cx);
        let body = cx.focus_handle();

        window.focus(&body, cx);

        Host {
            popup: overlays.popover(cx),
            dialog: overlays.dialog(cx),
            anchor: overlays.anchor(),
            overlays,
            body,
            trigger: cx.focus_handle(),
            other: cx.focus_handle(),
            popup_first: cx.focus_handle(),
            popup_last: cx.focus_handle(),
            dialog_first: cx.focus_handle(),
            dialog_last: cx.focus_handle(),
            disabled: cx.focus_handle(),
            show_anchor: true,
            show_popup: true,
            show_dialog: true,
            consume_tab: false,
            consume_escape: false,
            anchor_x: 20.,
            anchor_y: 80.,
            clicks: 0,
            actions: 0,
            scroll: gpui::ScrollHandle::new(),
            empty_dialog: false,
        }
    });

    cx.update_window(window.into(), |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    draw(&mut cx, window);

    (cx, window)
}

fn keys(cx: &mut gpui::TestAppContext, handle: gpui::WindowHandle<Host>, key: &str) {
    cx.update_window(handle.into(), |_, window, cx| {
        let keystroke = gpui::Keystroke::parse(key).unwrap();

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
    .unwrap();
    cx.run_until_parked();
    draw(cx, handle);
}

fn open_popup(cx: &mut gpui::TestAppContext, window: gpui::WindowHandle<Host>) {
    window
        .update(cx, |host, window, cx| {
            window.focus(&host.trigger, cx);
            assert!(host.popup.open(window, cx));
            assert!(
                !host.popup.open(window, cx),
                "opening twice must not duplicate stack entries"
            );
        })
        .unwrap();
    draw(cx, window);
}

fn open_dialog(cx: &mut gpui::TestAppContext, window: gpui::WindowHandle<Host>) {
    window
        .update(cx, |host, window, cx| {
            host.dialog.open(window, cx);
        })
        .unwrap();
    draw(cx, window);
}

#[test]
fn nested_escape_closes_only_the_top_and_restores_each_opener() {
    let (mut cx, window) = setup();

    open_popup(&mut cx, window);
    window
        .update(&mut cx, |host, window, _| {
            assert!(host.popup_first.is_focused(window))
        })
        .unwrap();
    keys(&mut cx, window, "enter");
    window
        .update(&mut cx, |host, window, _| {
            assert!(host.dialog_first.is_focused(window));
            assert_eq!(host.overlays.len(), 2);
        })
        .unwrap();
    keys(&mut cx, window, "escape");
    window
        .update(&mut cx, |host, window, _| {
            assert!(!host.dialog.is_open());
            assert!(host.popup.is_open());
            assert!(host.popup_first.is_focused(window));
        })
        .unwrap();
    keys(&mut cx, window, "escape");
    window
        .update(&mut cx, |host, window, _| {
            assert!(host.overlays.is_empty());
            assert!(host.trigger.is_focused(window));
        })
        .unwrap();
}

#[test]
fn modal_tab_wraps_skips_disabled_controls_and_handles_consumed_tab() {
    let (mut cx, window) = setup();

    open_dialog(&mut cx, window);
    keys(&mut cx, window, "shift-tab");
    window
        .update(&mut cx, |host, window, _| {
            assert!(host.dialog_last.is_focused(window))
        })
        .unwrap();
    keys(&mut cx, window, "tab");
    window
        .update(&mut cx, |host, window, cx| {
            assert!(
                host.dialog_first.is_focused(window),
                "focused={:?} first={:?} last={:?}",
                window.focused(cx),
                host.dialog_first,
                host.dialog_last
            )
        })
        .unwrap();
    keys(&mut cx, window, "tab");
    window
        .update(&mut cx, |host, window, cx| {
            assert!(host.dialog_last.is_focused(window));
            host.consume_tab = true;
            cx.notify();
        })
        .unwrap();
    draw(&mut cx, window);
    keys(&mut cx, window, "tab");
    window
        .update(&mut cx, |host, window, cx| {
            assert!(
                host.dialog_first.is_focused(window),
                "focused={:?} first={:?} last={:?}",
                window.focused(cx),
                host.dialog_first,
                host.dialog_last
            )
        })
        .unwrap();
    keys(&mut cx, window, "b");
    window
        .read_with(&cx, |host, _| {
            assert_eq!(
                host.actions, 0,
                "background bindings must be outside the modal dispatch path"
            )
        })
        .unwrap();
}

#[test]
fn consumed_escape_keeps_dialog_open_and_programmatic_focus_cannot_escape_modal() {
    let (mut cx, window) = setup();

    open_dialog(&mut cx, window);
    window
        .update(&mut cx, |host, _, cx| {
            host.consume_escape = true;
            cx.notify();
        })
        .unwrap();
    draw(&mut cx, window);
    keys(&mut cx, window, "escape");
    window
        .update(&mut cx, |host, window, cx| {
            assert!(host.dialog.is_open());
            window.focus(&host.other, cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(&mut cx, |host, window, cx| {
            assert!(host.dialog.focus_handle().contains_focused(window, cx))
        })
        .unwrap();
}

#[test]
fn anchors_follow_layout_and_popovers_fit_the_window() {
    let (mut cx, window) = setup();

    open_popup(&mut cx, window);
    let first = window
        .read_with(&cx, |host, _| host.popup.bounds().unwrap())
        .unwrap();

    window
        .update(&mut cx, |host, _, cx| {
            host.anchor_x += 30.;
            host.anchor_y += 25.;
            cx.notify();
        })
        .unwrap();
    draw(&mut cx, window);
    window
        .update(&mut cx, |host, window, _| {
            let next = host.popup.bounds().unwrap();

            assert_eq!(next.origin.x - first.origin.x, gpui::px(30.));
            assert_eq!(next.origin.y - first.origin.y, gpui::px(25.));
            host.anchor_x = f32::from(window.viewport_size().width) - 40.;
            host.anchor_y = f32::from(window.viewport_size().height) - 35.;
        })
        .unwrap();
    draw(&mut cx, window);
    window
        .update(&mut cx, |host, window, _| {
            let bounds = host.popup.bounds().unwrap();

            assert!(bounds.left() >= gpui::px(16.));
            assert!(bounds.top() >= gpui::px(16.));
            assert!(bounds.right() <= window.viewport_size().width - gpui::px(16.));
            assert!(bounds.bottom() <= window.viewport_size().height - gpui::px(16.));
        })
        .unwrap();
}

#[test]
fn removing_an_anchor_closes_its_popup_and_uses_a_mounted_focus_fallback() {
    let (mut cx, window) = setup();

    open_popup(&mut cx, window);
    window
        .update(&mut cx, |host, _, cx| {
            host.show_anchor = false;
            cx.notify();
        })
        .unwrap();
    draw(&mut cx, window);
    window
        .update(&mut cx, |host, window, _| {
            assert!(!host.popup.is_open());
            assert!(host.overlays.focus_handle().is_focused(window));
        })
        .unwrap();
}

#[test]
fn outside_click_dismisses_without_activating_background() {
    let (mut cx, handle) = setup();

    open_popup(&mut cx, handle);
    open_dialog(&mut cx, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        let position = gpui::point(gpui::px(5.), gpui::px(5.));

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
    .unwrap();
    draw(&mut cx, handle);
    handle
        .read_with(&cx, |host, _| {
            assert!(!host.dialog.is_open());
            assert!(
                host.popup.is_open(),
                "one click must dismiss only one layer"
            );
            assert_eq!(host.clicks, 0);
        })
        .unwrap();
}

#[test]
fn nonmodal_focus_can_leave_and_closing_a_parent_closes_descendants() {
    let (mut cx, window) = setup();

    open_popup(&mut cx, window);
    window
        .update(&mut cx, |host, window, cx| window.focus(&host.other, cx))
        .unwrap();
    cx.run_until_parked();
    draw(&mut cx, window);
    window
        .update(&mut cx, |host, window, _| {
            assert!(!host.popup.is_open());
            assert!(host.other.is_focused(window));
        })
        .unwrap();
    open_popup(&mut cx, window);
    open_dialog(&mut cx, window);
    window
        .update(&mut cx, |host, window, cx| {
            host.popup.close(window, cx);
        })
        .unwrap();
    draw(&mut cx, window);
    window
        .update(&mut cx, |host, window, _| {
            assert!(!host.dialog.is_open());
            assert!(host.overlays.is_empty());
            assert!(host.trigger.is_focused(window));
        })
        .unwrap();
}

#[test]
fn anchor_tracks_scroll_and_closes_when_scrolled_out_of_view() {
    let (mut cx, window) = setup();

    open_popup(&mut cx, window);
    let first = window
        .read_with(&cx, |host, _| host.popup.bounds().unwrap())
        .unwrap();

    window
        .update(&mut cx, |host, _, cx| {
            host.scroll
                .set_offset(gpui::point(gpui::px(0.), gpui::px(-30.)));
            cx.notify();
        })
        .unwrap();
    draw(&mut cx, window);
    window
        .read_with(&cx, |host, _| {
            assert_eq!(
                host.popup.bounds().unwrap().origin.y,
                first.origin.y - gpui::px(30.)
            );
        })
        .unwrap();
    window
        .update(&mut cx, |host, _, cx| {
            host.scroll
                .set_offset(gpui::point(gpui::px(0.), gpui::px(-500.)));
            cx.notify();
        })
        .unwrap();
    draw(&mut cx, window);
    window
        .read_with(&cx, |host, _| assert!(!host.popup.is_open()))
        .unwrap();
}

#[test]
fn empty_dialog_keeps_focus_and_unmounting_a_surface_closes_it() {
    let (mut cx, window) = setup();

    window
        .update(&mut cx, |host, _, cx| {
            host.empty_dialog = true;
            cx.notify();
        })
        .unwrap();
    open_dialog(&mut cx, window);

    for key in ["tab", "shift-tab"] {
        keys(&mut cx, window, key);
        window
            .update(&mut cx, |host, window, _| {
                assert!(host.dialog.focus_handle().is_focused(window))
            })
            .unwrap();
    }

    window
        .update(&mut cx, |host, _, cx| {
            host.show_dialog = false;
            cx.notify();
        })
        .unwrap();
    draw(&mut cx, window);
    window
        .read_with(&cx, |host, _| assert!(!host.dialog.is_open()))
        .unwrap();
}

#[test]
fn tab_can_leave_a_nonmodal_popover_in_both_directions() {
    let (mut cx, window) = setup();

    open_popup(&mut cx, window);
    keys(&mut cx, window, "shift-tab");
    window
        .update(&mut cx, |host, window, _| {
            assert!(!host.popup.is_open());
            assert!(host.other.is_focused(window));
        })
        .unwrap();

    open_popup(&mut cx, window);
    keys(&mut cx, window, "tab");
    window
        .update(&mut cx, |host, window, _| {
            assert!(host.popup_last.is_focused(window))
        })
        .unwrap();
    keys(&mut cx, window, "tab");
    window
        .update(&mut cx, |host, window, _| {
            assert!(!host.popup.is_open());
            assert!(host.trigger.is_focused(window));
        })
        .unwrap();
}
