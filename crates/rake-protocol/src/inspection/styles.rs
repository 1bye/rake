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

/// Reads solid sRGB backgrounds, GPUI hex colors and legacy palette HSL metadata.
/// Unknown fills, color spaces and invalid channel values return `None`.
#[must_use]
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

    if let Some(hex) = value.as_str() {
        return hex_color(hex);
    }

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

fn hex_color(value: &str) -> Option<Color> {
    let digits = value.strip_prefix('#')?;

    if !matches!(digits.len(), 6 | 8) || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }

    let channel = |start| {
        u8::from_str_radix(&digits[start..start + 2], 16)
            .ok()
            .map(|v| f32::from(v) / 255.)
    };

    let (r, g, b) = (channel(0)?, channel(2)?, channel(4)?);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let l = (max + min) / 2.;

    let (h, s) = if delta == 0. {
        (0., 0.)
    } else {
        let sector = if max == r {
            ((g - b) / delta).rem_euclid(6.)
        } else if max == g {
            (b - r) / delta + 2.
        } else {
            (r - g) / delta + 4.
        };

        (sector / 6., delta / (1. - (2. * l - 1.).abs()))
    };

    Some(Color {
        h,
        s,
        l,
        a: if digits.len() == 8 { channel(6)? } else { 1. },
    })
}

impl Color {
    /// Returns an sRGB hex color, preserving alpha when translucent.
    #[must_use]
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

#[cfg(test)]
mod tests {
    use super::color;
    use serde_json::json;

    #[test]
    fn gpui_hex_colors_preserve_text_background_and_alpha() {
        for (value, expected) in [
            (json!("#ff0000ff"), "#ff0000"),
            (json!("#00ff0080"), "#00ff0080"),
            (
                json!({"Color":{"tag":"Solid","color_space":"Srgb","solid":"#29424bff"}}),
                "#29424b",
            ),
        ] {
            assert_eq!(
                color(&value).map(|color| color.css()).as_deref(),
                Some(expected),
                "{value}"
            );
        }
    }

    #[test]
    fn unsupported_and_malformed_colors_stay_unavailable() {
        for value in [
            json!("#bad"),
            json!("#gg0000ff"),
            json!("red"),
            json!({"Color":{"tag":"LinearGradient","solid":"#ff0000ff"}}),
            json!({"Color":{"tag":"Solid","color_space":"Oklab","solid":"#ff0000ff"}}),
        ] {
            assert!(color(&value).is_none(), "{value}");
        }
    }
}
