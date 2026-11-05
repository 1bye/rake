//! GPUI-free input state shared by the native controller and GPU frame snapshot.

use std::collections::BTreeSet;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(crate) struct InputState {
    pub position: [f32; 2],
    pub has_pointer: bool,
    pub pressed: bool,
    pub focused: bool,
    pub zoom: f32,
    pub wheel_px: [f32; 2],
    pub downs: u64,
    pub moves: u64,
    pub ups: u64,
    pub wheels: u64,
    pub key_downs: u64,
    pub key_ups: u64,
    pub repeats: u64,
    pub cancellations: u64,
    pub keys: BTreeSet<String>,
    pub last_key: String,
    pub text: Arc<super::TextInput>,
}

impl Default for InputState {
    fn default() -> Self {
        Self {
            position: [0.; 2],
            has_pointer: false,
            pressed: false,
            focused: false,
            zoom: 1.,
            wheel_px: [0.; 2],
            downs: 0,
            moves: 0,
            ups: 0,
            wheels: 0,
            key_downs: 0,
            key_ups: 0,
            repeats: 0,
            cancellations: 0,
            keys: BTreeSet::new(),
            last_key: String::new(),
            text: Arc::default(),
        }
    }
}

impl InputState {
    pub fn pointer_move(&mut self, x: f32, y: f32) {
        if self.position(x, y) {
            self.moves += 1;
        }
    }

    pub fn pointer_down(&mut self, x: f32, y: f32) {
        if self.focused && self.position(x, y) {
            self.pressed = true;
            self.downs += 1;
        }
    }

    pub fn pointer_up(&mut self, x: f32, y: f32) {
        if self.pressed && self.position(x, y) {
            self.pressed = false;
            self.ups += 1;
        }
    }

    pub fn diagnostics(&self) -> String {
        let status = format!(
            "focus={} drag={} pointer={}/{} move={} wheel={} key={}/{} held={} xy=({:.0},{:.0}) zoom={:.2} cancel={} repeat={}",
            self.focused,
            self.pressed,
            self.downs,
            self.ups,
            self.moves,
            self.wheels,
            self.key_downs,
            self.key_ups,
            self.keys.len(),
            self.position[0],
            self.position[1],
            self.zoom,
            self.cancellations,
            self.repeats,
        );
        let status = format!("{status} {}", self.text.diagnostics());
        status
    }

    pub fn position(&mut self, x: f32, y: f32) -> bool {
        if !x.is_finite() || !y.is_finite() {
            return false;
        }

        self.position = [x, y];
        self.has_pointer = true;

        true
    }

    pub fn focus(&mut self, focused: bool) {
        self.focused = focused;

        if !focused {
            self.cancel();
        }
    }

    pub fn cancel(&mut self) {
        if self.pressed || !self.keys.is_empty() {
            self.cancellations += 1;
        }

        self.pressed = false;
        self.keys.clear();
    }

    pub fn wheel(&mut self, dx: f32, dy: f32) {
        if !dx.is_finite() || !dy.is_finite() {
            return;
        }

        self.wheels += 1;
        self.wheel_px[0] = (self.wheel_px[0] + dx).clamp(-1e6, 1e6);
        self.wheel_px[1] = (self.wheel_px[1] + dy).clamp(-1e6, 1e6);
        self.zoom = (self.zoom * (dy / 600.).exp()).clamp(0.5, 3.);
    }

    pub fn key_down(&mut self, key: &str, repeat: bool) {
        if !self.focused {
            return;
        }

        self.key_downs += 1;
        self.repeats += u64::from(repeat);
        self.keys.insert(key.into());
        self.last_key = key.into();

        match key {
            "left" => self.position[0] -= 20.,
            "right" => self.position[0] += 20.,
            "up" => self.position[1] -= 20.,
            "down" => self.position[1] += 20.,
            _ => {}
        }
    }

    pub fn key_up(&mut self, key: &str) {
        if self.focused && self.keys.remove(key) {
            self.key_ups += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::InputState;

    #[test]
    fn blur_cancels_drag_and_keys_without_accepting_orphan_releases() {
        let mut input = InputState::default();
        input.key_down("a", false);
        assert_eq!(
            input.key_downs, 0,
            "unfocused keyboard input must be ignored"
        );
        input.focus(true);
        input.pressed = true;
        input.key_down("right", false);
        input.key_down("right", true);
        assert_eq!(input.position[0], 40.);
        assert_eq!(input.repeats, 1);

        input.focus(false);
        input.key_up("right");

        assert!(!input.pressed);
        assert!(input.keys.is_empty());
        assert_eq!(input.cancellations, 1);
        assert_eq!(
            input.key_ups, 0,
            "a release after cancellation must not become input"
        );
    }

    #[test]
    fn physical_coordinates_survive_capture_and_invalid_values_do_not_reach_gpu() {
        let mut input = InputState::default();
        assert!(input.position(-40., 900.));
        assert!(!input.position(f32::NAN, 2.));
        assert_eq!(input.position, [-40., 900.]);
        input.wheel(0., 600.);
        assert!((input.zoom - std::f32::consts::E).abs() < 1e-5);
        input.wheel(0., f32::INFINITY);
        assert_eq!(input.wheels, 1);
        input.wheel(0., f32::MAX);
        assert_eq!(input.zoom, 3.);
    }
}
