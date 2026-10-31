//! Typed application of app-owned theme fields, used by generated code.

use gpui::{Fill, Hsla, Pixels, Rems, SharedString, Styled};
use palette::IntoColor;

use super::TextToken;

// Separate inference markers keep the blanket color conversion implementation
// disjoint from TextToken without restricting callers to two concrete colors.
/// Inference marker distinguishing colors from typography tokens.
pub enum Color {}

/// Inference marker for paragraph typography.
pub enum Typography {}

/// Applies a theme color or a paragraph typography token.
pub trait ThemeText<Kind> {
    /// Applies this value to a GPUI style builder.
    fn apply<S: Styled>(self, style: S) -> S;
}

impl<T: IntoColor<Hsla>> ThemeText<Color> for T {
    fn apply<S: Styled>(self, style: S) -> S {
        style.text_color(self)
    }
}

impl ThemeText<Typography> for TextToken {
    fn apply<S: Styled>(self, style: S) -> S {
        let mut style = style.text_size(self.size);

        if let Some(height) = self.line_height {
            style = style.line_height(height);
        }

        if let Some(weight) = self.weight {
            style = style.font_weight(weight);
        }

        style
    }
}

/// Applies a theme background, with GPUI's native fill conversion.
pub trait ThemeBackground {
    /// Applies this value to a GPUI style builder.
    fn apply<S: Styled>(self, style: S) -> S;
}

impl<T: Into<Fill>> ThemeBackground for T {
    fn apply<S: Styled>(self, style: S) -> S {
        style.bg(self)
    }
}

/// Applies a theme border color.
pub trait ThemeBorder {
    /// Applies this value to a GPUI style builder.
    fn apply<S: Styled>(self, style: S) -> S;
}

impl<T: IntoColor<Hsla>> ThemeBorder for T {
    fn apply<S: Styled>(self, style: S) -> S {
        style.border_color(self)
    }
}

/// Applies a theme font family.
pub trait ThemeFont {
    /// Applies this value to a GPUI style builder.
    fn apply<S: Styled>(self, style: S) -> S;
}

impl ThemeFont for SharedString {
    fn apply<S: Styled>(self, style: S) -> S {
        style.font_family(self)
    }
}

impl ThemeFont for &'static str {
    fn apply<S: Styled>(self, style: S) -> S {
        style.font_family(self)
    }
}

/// Applies an absolute theme corner radius.
pub trait ThemeRadius {
    /// Applies this value to a GPUI style builder.
    fn apply<S: Styled>(self, style: S) -> S;
}

impl ThemeRadius for Pixels {
    fn apply<S: Styled>(self, style: S) -> S {
        style.rounded(self)
    }
}

impl ThemeRadius for Rems {
    fn apply<S: Styled>(self, style: S) -> S {
        style.rounded(self)
    }
}

/// Applies an inline theme color. Typography size belongs on p; TextToken is
/// deliberately unsupported by this trait.
pub trait ThemeRunText {
    /// Applies only a run's foreground color.
    fn apply(self, style: crate::view::RunStyle) -> crate::view::RunStyle;
}

impl<T: IntoColor<Hsla>> ThemeRunText for T {
    fn apply(self, style: crate::view::RunStyle) -> crate::view::RunStyle {
        style.highlight(gpui::HighlightStyle::color(self.into_color()))
    }
}

/// Applies an inline theme font family without changing paragraph size.
pub trait ThemeRunFont {
    /// Applies a run's family.
    fn apply(self, style: crate::view::RunStyle) -> crate::view::RunStyle;
}

impl ThemeRunFont for SharedString {
    fn apply(self, style: crate::view::RunStyle) -> crate::view::RunStyle {
        style.font_family(self)
    }
}

impl ThemeRunFont for &'static str {
    fn apply(self, style: crate::view::RunStyle) -> crate::view::RunStyle {
        style.font_family(self)
    }
}
