//! Portable conversion of captured GPUI color metadata; no renderer or CSS cascade.

use serde_json::Value;

/// An sRGB HSL color with channels normalized to 0–1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    /// Hue, as a fraction of one turn.
    pub h: f32,
    /// Saturation.
    pub s: f32,
    /// Lightness.
    pub l: f32,
    /// Alpha.
    pub a: f32,
}

/// Reads solid sRGB backgrounds or palette HSL colors from captured style JSON.
/// Unknown fills, color spaces and invalid channel values return `None`.
pub fn color(value: &Value) -> Option<Color> {
    let (value, names, hue_scale) = if value.get("Color").is_some() {
        let color = &value["Color"];

        if color["tag"] != "Solid"
            || color
                .get("color_space")
                .is_some_and(|space| space != "Srgb")
        {
            return None;
        }

        (&color["solid"], ["h", "s", "l", "a"], 1.)
    } else {
        (value, ["hue", "saturation", "lightness", "alpha"], 360.)
    };

    let values: Option<Vec<_>> = names.into_iter().map(|name| value[name].as_f64()).collect();
    let values = values?;

    if !(0.0..=hue_scale).contains(&values[0])
        || values[1..].iter().any(|value| !(0.0..=1.0).contains(value))
    {
        return None;
    }

    Some(Color {
        h: (values[0] / hue_scale) as f32,
        s: values[1] as f32,
        l: values[2] as f32,
        a: values[3] as f32,
    })
}

impl Color {
    /// Returns an sRGB hex color, preserving alpha when translucent.
    pub fn css(self) -> String {
        let color = self;
        let chroma = (1. - (2. * color.l - 1.).abs()) * color.s;
        let sector = color.h * 6.;
        let second = chroma * (1. - (sector % 2. - 1.).abs());

        let (r, g, b) = match sector as u8 {
            0 | 6 => (chroma, second, 0.),
            1 => (second, chroma, 0.),
            2 => (0., chroma, second),
            3 => (0., second, chroma),
            4 => (second, 0., chroma),
            _ => (chroma, 0., second),
        };

        let offset = color.l - chroma / 2.;
        let channel = |value: f32| ((value + offset) * 255.).round() as u8;
        let mut value = format!("#{:02x}{:02x}{:02x}", channel(r), channel(g), channel(b));

        if color.a < 1. {
            value.push_str(&format!("{:02x}", (color.a * 255.).round() as u8));
        }

        value
    }
}
