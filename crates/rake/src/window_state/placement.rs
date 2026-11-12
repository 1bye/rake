//! Portable validation, versioned storage, and display placement in logical pixels.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub(super) fn valid(self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|value| value.is_finite() && value.abs() <= 1_000_000.)
            && self.width > 0.
            && self.height > 0.
    }

    fn intersects(self, other: Self) -> bool {
        self.x < other.x + other.width
            && self.x + self.width > other.x
            && self.y < other.y + other.height
            && self.y + self.height > other.y
    }

    fn clamp(self, display: Self) -> Self {
        let width = self.width.min(display.width);
        let height = self.height.min(display.height);

        Self {
            x: self.x.clamp(display.x, display.x + display.width - width),
            y: self.y.clamp(display.y, display.y + display.height - height),
            width,
            height,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Mode {
    Windowed,
    Maximized,
    Fullscreen,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct SavedWindow {
    pub bounds: Rect,
    pub display: Option<String>,
    pub mode: Mode,
}

#[derive(Debug)]
#[cfg_attr(
    not(all(feature = "gpui", not(target_family = "wasm"))),
    allow(dead_code)
)]
pub(super) struct Display {
    pub uuid: Option<String>,
    pub visible: Rect,
}

pub(super) fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 128
}

pub(super) fn decode(value: &str) -> BTreeMap<String, SavedWindow> {
    #[derive(Deserialize)]
    struct Document {
        version: u32,
        windows: BTreeMap<String, serde_json::Value>,
    }

    let Ok(document) = serde_json::from_str::<Document>(value) else {
        return BTreeMap::new();
    };

    if document.version != 1 {
        return BTreeMap::new();
    }

    document
        .windows
        .into_iter()
        .filter_map(|(key, value)| {
            let record = serde_json::from_value::<SavedWindow>(value).ok()?;

            (valid_key(&key) && record.bounds.valid()).then_some((key, record))
        })
        .collect()
}

pub(super) fn encode(windows: &BTreeMap<String, SavedWindow>) -> String {
    serde_json::json!({"version": 1, "windows": windows}).to_string()
}

/// Returns a display index and fully visible restore bounds. The caller keeps
/// its defaults if no valid display exists. A missing/off-screen display uses
/// the app's default display and placement, clamped to that display's work area.
#[cfg_attr(
    not(all(feature = "gpui", not(target_family = "wasm"))),
    allow(dead_code)
)]
pub(super) fn restore(
    saved: &SavedWindow,
    displays: &[Display],
    default_display: usize,
    default_bounds: Rect,
) -> Option<(usize, Rect)> {
    if !saved.bounds.valid() {
        return None;
    }

    let selected = displays.iter().position(|display| {
        display.visible.valid() && saved.display.is_some() && display.uuid == saved.display
    });

    let selected = selected.or_else(|| {
        saved.display.is_none().then(|| {
            displays.iter().position(|display| {
                display.visible.valid() && saved.bounds.intersects(display.visible)
            })
        })?
    });

    if let Some(index) = selected
        && saved.bounds.intersects(displays[index].visible)
    {
        return Some((index, saved.bounds.clamp(displays[index].visible)));
    }

    let index = displays
        .get(default_display)
        .filter(|display| display.visible.valid())
        .map(|_| default_display)
        .or_else(|| displays.iter().position(|display| display.visible.valid()))?;

    let display = displays[index].visible;
    let fallback = if default_bounds.valid() {
        default_bounds
    } else {
        saved.bounds
    };
    let bounds = if fallback.intersects(display) {
        fallback
    } else {
        Rect {
            x: display.x + (display.width - fallback.width.min(display.width)) / 2.,
            y: display.y + (display.height - fallback.height.min(display.height)) / 2.,
            ..fallback
        }
    };

    Some((index, bounds.clamp(display)))
}

#[cfg(test)]
mod tests;
