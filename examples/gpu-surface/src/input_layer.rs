//! GPUI focus, hit testing and input ownership for the macOS producer.
use rake::{
    gpui::{self, App, FocusHandle, HitboxBehavior, MouseButton, Window, prelude::*},
    overlay::OverlayState,
};
use std::{cell::Cell, rc::Rc};
use surface_spike_renderer::NativeInputController;

mod ime;

struct Bridge {
    controller: NativeInputController,
    focus: FocusHandle,
    fallback: FocusHandle,
    visible: Cell<bool>,
    focused: Cell<bool>,
    dragging: Cell<bool>,
    dialog: OverlayState,
    popup: OverlayState,
    text_mode: Cell<bool>,
}

impl Bridge {
    fn allowed(&self, window: &Window) -> bool {
        self.visible.get()
            && window.is_window_active()
            && !self.dialog.is_open()
            && !self.popup.is_open()
    }

    fn owns_focus(&self, window: &Window) -> bool {
        self.allowed(window) && self.focus.is_focused(window)
    }

    fn accepts_text(&self, window: &Window) -> bool {
        self.text_mode.get() && self.owns_focus(window)
    }

    fn focus(&self, focused: bool) {
        if self.focused.replace(focused) != focused {
            self.controller.set_focus(focused);
        }
    }

    fn cancel(&self, window: &mut Window) {
        self.focus(false);

        if self.dragging.replace(false) {
            window.release_pointer();
        }
    }
}

pub(super) struct InputLayer {
    bridge: Rc<Bridge>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl InputLayer {
    pub(super) fn new(
        controller: NativeInputController,
        window: &mut Window,
        cx: &mut App,
        fallback: FocusHandle,
        dialog: OverlayState,
        popup: OverlayState,
    ) -> Rc<Self> {
        let bridge = Rc::new(Bridge {
            controller,
            focus: cx.focus_handle().tab_stop(true),
            fallback,
            visible: Cell::new(false),
            focused: Cell::new(false),
            dragging: Cell::new(false),
            dialog,
            popup,
            text_mode: Cell::new(false),
        });
        let entered = bridge.clone();
        let exited = bridge.clone();
        let subscriptions = vec![
            window.on_focus_in(&bridge.focus, cx, move |window, _| {
                entered.focus(entered.allowed(window))
            }),
            window.on_focus_out(&bridge.focus, cx, move |_, window, _| exited.cancel(window)),
        ];

        Rc::new(Self {
            bridge,
            _subscriptions: subscriptions,
        })
    }

    pub(super) fn set_visible(&self, visible: bool) {
        self.bridge.visible.set(visible);

        if !visible {
            self.bridge.focus(false);
        }
    }

    pub(super) fn stop(&self) {
        self.bridge.focus(false);
    }

    pub(super) fn cancel(&self, window: &mut Window) {
        self.bridge.cancel(window);
    }

    pub(super) fn text_mode(&self) -> bool {
        self.bridge.text_mode.get()
    }

    pub(super) fn toggle_text_mode(&self, window: &mut Window) {
        self.bridge.cancel(window);
        self.bridge.text_mode.set(!self.text_mode());
    }

    pub(super) fn accepts_text(&self, window: &Window) -> bool {
        self.bridge.accepts_text(window)
    }

    pub(super) fn element(
        &self,
        enabled: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> gpui::AnyElement {
        let bridge = &self.bridge;

        if !enabled || !bridge.allowed(window) {
            bridge.cancel(window);
            if !enabled && bridge.focus.is_focused(window) {
                window.focus(&bridge.fallback, cx);
            }
        } else {
            bridge.focus(bridge.focus.is_focused(window));
        }

        if !enabled {
            return gpui::Empty.into_any_element();
        }

        let pointer = bridge.clone();
        let keys_down = bridge.clone();
        let keys_up = bridge.clone();
        let outside = bridge.clone();

        gpui::div()
            .id("renderer-input")
            .absolute()
            .inset_0()
            .track_focus(&bridge.focus)
            .on_mouse_down_out(move |_, window, cx| {
                if outside.focus.is_focused(window) {
                    outside.cancel(window);
                    window.focus(&outside.fallback, cx);
                }
            })
            .on_key_down(move |event, window, cx| {
                let stroke = &event.keystroke;
                if keys_down.accepts_text(window) {
                    if stroke.key == "backspace"
                        && !stroke.modifiers.modified()
                        && keys_down.controller.text().marked_range().is_none()
                    {
                        keys_down.controller.backspace();
                        window.prevent_default();
                        cx.stop_propagation();
                    }
                    return;
                }

                if keys_down.owns_focus(window)
                    && !stroke.modifiers.platform
                    && !stroke.modifiers.control
                    && !stroke.modifiers.alt
                    && !reserved_key(&stroke.key)
                {
                    keys_down.controller.key_down(&stroke.key, event.is_held);
                    window.prevent_default();
                    cx.stop_propagation();
                }
            })
            .on_key_up(move |event, window, cx| {
                if keys_up.accepts_text(window) {
                    return;
                }

                if keys_up.owns_focus(window) && !reserved_key(&event.keystroke.key) {
                    keys_up.controller.key_up(&event.keystroke.key);
                    cx.stop_propagation();
                }
            })
            .child(
                gpui::canvas(
                    |bounds, window, _| input_hitbox(bounds, window),
                    move |bounds, hitbox, window, cx| {
                        if pointer.accepts_text(window) {
                            window.handle_input(
                                &pointer.focus,
                                ime::ImeInput {
                                    bridge: pointer.clone(),
                                    viewport: hitbox.bounds.intersect(&hitbox.content_mask.bounds),
                                },
                                cx,
                            );
                        }

                        // Hitbox IDs change every frame; transfer an owned drag to this frame's box.
                        if pointer.dragging.get() && pointer.allowed(window) {
                            window.capture_pointer(hitbox.id);
                        }
                        let down = pointer.clone();
                        let down_box = hitbox.clone();
                        window.on_mouse_event(
                            move |event: &gpui::MouseDownEvent, phase, window, cx| {
                                if phase.bubble()
                                    && event.button == MouseButton::Left
                                    && down_box.is_hovered(window)
                                    && down.allowed(window)
                                {
                                    window.focus(&down.focus, cx);
                                    down.focus(true);
                                    down.dragging.set(true);
                                    window.capture_pointer(down_box.id);
                                    let position =
                                        (event.position - bounds.origin) * window.scale_factor();
                                    down.controller
                                        .pointer_down(position.x.as_f32(), position.y.as_f32());
                                    window.prevent_default();
                                    cx.stop_propagation();
                                }
                            },
                        );
                        let moved = pointer.clone();
                        let move_box = hitbox.clone();
                        window.on_mouse_event(
                            move |event: &gpui::MouseMoveEvent, phase, window, cx| {
                                if phase.bubble()
                                    && move_box.is_hovered(window)
                                    && moved.allowed(window)
                                {
                                    let position =
                                        (event.position - bounds.origin) * window.scale_factor();
                                    moved
                                        .controller
                                        .pointer_move(position.x.as_f32(), position.y.as_f32());
                                    cx.stop_propagation();
                                }
                            },
                        );
                        let up = pointer.clone();
                        let up_box = hitbox.clone();
                        window.on_mouse_event(
                            move |event: &gpui::MouseUpEvent, phase, window, cx| {
                                if phase.bubble()
                                    && event.button == MouseButton::Left
                                    && up.dragging.get()
                                    && up_box.is_hovered(window)
                                    && up.allowed(window)
                                {
                                    let position =
                                        (event.position - bounds.origin) * window.scale_factor();
                                    up.controller
                                        .pointer_up(position.x.as_f32(), position.y.as_f32());
                                    up.dragging.set(false);
                                    window.release_pointer();
                                    cx.stop_propagation();
                                }
                            },
                        );
                        window.on_mouse_event(
                            move |event: &gpui::ScrollEvent, phase, window, cx| {
                                if phase.bubble()
                                    && hitbox.is_hovered(window)
                                    && pointer.allowed(window)
                                    && !event.modifiers.platform
                                    && !event.modifiers.control
                                    && !event.modifiers.alt
                                {
                                    let delta = event.delta.pixel_delta(gpui::px(16.))
                                        * window.scale_factor();
                                    pointer.controller.wheel(delta.x.as_f32(), delta.y.as_f32());
                                    window.prevent_default();
                                    cx.stop_propagation();
                                }
                            },
                        );
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }
}

fn input_hitbox(bounds: gpui::Bounds<gpui::Pixels>, window: &mut Window) -> gpui::Hitbox {
    // Route AppKit input to GPUI above the producer's composition surface.
    let overlay = window
        .enable_window_composition()
        .expect("surface composition enabled")
        .overlay_surface();
    window
        .with_composition_surface(overlay, |window| {
            window.insert_hitbox(bounds, HitboxBehavior::BlockMouse)
        })
        .expect("GPUI overlay surface accepts input regions")
}

fn reserved_key(key: &str) -> bool {
    matches!(key, "tab" | "escape" | "h" | "r" | "b" | "m")
}
