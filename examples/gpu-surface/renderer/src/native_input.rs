//! Bounded native input state; GPU work never runs while this mutex is held.

use super::input::InputState;
use std::ops::Range;
use std::sync::{Arc, Mutex};

/// Thread-safe input endpoint; the producer snapshots it before drawing a frame.
#[derive(Clone, Default)]
pub struct NativeInputController(Arc<Mutex<InputState>>);

impl NativeInputController {
    fn edit(&self, update: impl FnOnce(&mut InputState)) {
        update(&mut self.0.lock().expect("input-state mutation must not panic"));
    }

    /// Moves the marker in full-viewport physical pixels.
    pub fn pointer_move(&self, x: f32, y: f32) {
        self.edit(|s| s.pointer_move(x, y));
    }

    /// Begins a focused primary-button drag.
    pub fn pointer_down(&self, x: f32, y: f32) {
        self.edit(|s| s.pointer_down(x, y));
    }

    /// Releases an owned drag, ignoring orphan releases.
    pub fn pointer_up(&self, x: f32, y: f32) {
        self.edit(|s| s.pointer_up(x, y));
    }

    /// Changes zoom from GPUI-normalized physical-pixel wheel deltas.
    pub fn wheel(&self, dx: f32, dy: f32) {
        self.edit(|s| s.wheel(dx, dy));
    }

    /// Accepts a GPUI key name and repeat flag while focused.
    /// A repeat requires a press received during the current focus session.
    pub fn key_down(&self, key: &str, repeat: bool) {
        self.edit(|s| {
            if !repeat || s.keys.contains(key) {
                s.key_down(key, repeat);
            }
        });
    }

    /// Releases a key previously delivered to this controller.
    pub fn key_up(&self, key: &str) {
        self.edit(|s| s.key_up(key));
    }

    /// Transfers logical focus. Loss cancels held buttons, keys and marked text.
    pub fn set_focus(&self, focused: bool) {
        self.edit(|s| {
            s.focus(focused);
            if !focused && s.text.marked_range().is_some() {
                Arc::make_mut(&mut s.text).cancel();
            }
        });
    }

    /// Returns the current text snapshot for AppKit's UTF-16 queries.
    pub fn text(&self) -> Arc<super::TextInput> {
        self.0
            .lock()
            .expect("input-state mutation must not panic")
            .text
            .clone()
    }

    /// Replaces the requested range, marked range or selection with committed text.
    pub fn replace_text(&self, range: Option<Range<usize>>, text: &str) {
        self.edit_text(|state| state.replace(range, text));
    }

    /// Updates composition using a UTF-16 replacement and a selection within the new text.
    pub fn mark_text(
        &self,
        range: Option<Range<usize>>,
        text: &str,
        selection: Option<Range<usize>>,
    ) {
        self.edit_text(|state| state.mark(range, text, selection));
    }

    /// Finalizes the current marked text.
    pub fn unmark_text(&self) {
        self.edit_text(super::TextInput::unmark);
    }

    /// Deletes the selection or preceding Unicode scalar outside composition.
    pub fn backspace(&self) {
        self.edit_text(super::TextInput::backspace);
    }

    fn edit_text(&self, update: impl FnOnce(&mut super::TextInput)) {
        self.edit(|s| {
            if s.focused {
                update(Arc::make_mut(&mut s.text));
            }
        });
    }

    /// Reports validated input for the native demo's status line.
    pub fn diagnostics(&self) -> String {
        self.snapshot().diagnostics()
    }

    pub(crate) fn snapshot(&self) -> InputState {
        self.0
            .lock()
            .expect("input-state mutation must not panic")
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::NativeInputController;

    #[test]
    fn producer_thread_observes_ordered_input_and_focus_cancellation() {
        let controller = NativeInputController::default();
        controller.set_focus(true);
        controller.pointer_down(40., 80.);
        controller.key_down("right", false);
        controller.wheel(0., 120.);
        let producer = controller.clone();

        let state = std::thread::spawn(move || producer.snapshot())
            .join()
            .expect("producer snapshot");

        assert_eq!(state.position, [60., 80.]);
        assert!(state.pressed);
        assert_eq!(state.key_downs, 1);
        assert_eq!(state.wheels, 1);
        controller.set_focus(false);
        controller.pointer_up(60., 80.);
        controller.key_up("right");
        let cancelled = controller.snapshot();
        assert!(!cancelled.pressed);
        assert!(cancelled.keys.is_empty());
        assert_eq!(cancelled.ups, 0);
        assert_eq!(cancelled.key_ups, 0);
        assert_eq!(cancelled.cancellations, 1);
    }

    #[test]
    fn repeat_cannot_reacquire_a_key_held_before_focus() {
        let controller = NativeInputController::default();
        controller.set_focus(true);
        controller.key_down("space", false);
        controller.key_down("space", true);
        controller.set_focus(false);
        controller.set_focus(true);

        controller.key_down("space", true);
        controller.key_up("space");

        let state = controller.snapshot();
        assert_eq!((state.key_downs, state.repeats, state.key_ups), (2, 1, 0));
        assert!(state.keys.is_empty());
    }

    #[test]
    fn utf16_composition_replaces_its_draft_and_rolls_back_on_blur() {
        let controller = NativeInputController::default();
        controller.set_focus(true);
        controller.replace_text(None, "A🙂B");
        controller.mark_text(Some(1..3), "か🙂", Some(1..3));

        let draft = controller.text();
        assert_eq!(draft.marked_range(), Some(1..4));
        assert_eq!(draft.selection(), 2..4);
        assert_eq!(draft.text_for_range(3..4), ("🙂", 2..4));
        controller.mark_text(None, "仮名", None);
        controller.replace_text(None, "かな");
        assert_eq!(controller.text().text_for_range(0..10), ("AかなB", 0..4));
        assert_eq!(controller.text().marked_range(), None);

        controller.mark_text(Some(1..3), "draft", None);
        controller.set_focus(false);
        controller.replace_text(None, "stale commit");

        let cancelled = controller.text();
        assert_eq!(cancelled.text_for_range(0..10), ("AかなB", 0..4));
        assert_eq!(cancelled.marked_range(), None);
    }
}
