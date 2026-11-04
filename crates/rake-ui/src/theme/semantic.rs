//! Flat semantic colors shared by Rake and Base.

use gpui::{App, Hsla};
use gpui_base::{ColorTokens, Theme};

use super::{Appearance, ThemeGlobal};

/// Flat Base color roles for `view!` and application-owned controls.
///
/// Install through [`super::install`]. Existing application-specific theme
/// structs remain independent. Typography, spacing and elevation remain Base's
/// separate scales; this bridge changes semantic colors and appearance only.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Semantic {
    /// Light or dark presentation, also used for Kit's remaining component colors.
    pub appearance: Appearance,
    /// Base’s `background` color role.
    pub background: Hsla,
    /// Base’s `foreground` color role.
    pub foreground: Hsla,
    /// Base’s `surface` color role.
    pub surface: Hsla,
    /// Base’s `surface_foreground` color role.
    pub surface_foreground: Hsla,
    /// Base’s `primary` color role.
    pub primary: Hsla,
    /// Base’s `primary_foreground` color role.
    pub primary_foreground: Hsla,
    /// Base’s `secondary` color role.
    pub secondary: Hsla,
    /// Base’s `secondary_foreground` color role.
    pub secondary_foreground: Hsla,
    /// Base’s `muted` color role.
    pub muted: Hsla,
    /// Base’s `muted_foreground` color role.
    pub muted_foreground: Hsla,
    /// Base’s `accent` color role.
    pub accent: Hsla,
    /// Base’s `accent_foreground` color role.
    pub accent_foreground: Hsla,
    /// Base’s `destructive` color role.
    pub destructive: Hsla,
    /// Base’s `destructive_foreground` color role.
    pub destructive_foreground: Hsla,
    /// Base’s `border` color role.
    pub border: Hsla,
    /// Base’s `input` color role.
    pub input: Hsla,
    /// Base’s `ring` color role.
    pub ring: Hsla,
    /// Base’s `selection` color role.
    pub selection: Hsla,
}

impl Default for Semantic {
    fn default() -> Self {
        Self::light()
    }
}

impl Semantic {
    /// Base's default light semantic palette.
    pub fn light() -> Self {
        Self::from_base(&Theme::default())
    }

    /// Base's default dark semantic palette.
    pub fn dark() -> Self {
        let mut base = Theme::default();

        base.appearance = Appearance::Dark;
        base.tokens.colors = ColorTokens::dark();

        Self::from_base(&base)
    }

    fn from_base(base: &Theme) -> Self {
        let colors = base.tokens.colors;

        Self {
            appearance: base.appearance,
            background: colors.background,
            foreground: colors.foreground,
            surface: colors.surface,
            surface_foreground: colors.surface_foreground,
            primary: colors.primary,
            primary_foreground: colors.primary_foreground,
            secondary: colors.secondary,
            secondary_foreground: colors.secondary_foreground,
            muted: colors.muted,
            muted_foreground: colors.muted_foreground,
            accent: colors.accent,
            accent_foreground: colors.accent_foreground,
            destructive: colors.destructive,
            destructive_foreground: colors.destructive_foreground,
            border: colors.border,
            input: colors.input,
            ring: colors.ring,
            selection: colors.selection,
        }
    }

    fn colors(&self) -> ColorTokens {
        let mut colors = ColorTokens::default();

        colors.background = self.background;
        colors.foreground = self.foreground;
        colors.surface = self.surface;
        colors.surface_foreground = self.surface_foreground;
        colors.primary = self.primary;
        colors.primary_foreground = self.primary_foreground;
        colors.secondary = self.secondary;
        colors.secondary_foreground = self.secondary_foreground;
        colors.muted = self.muted;
        colors.muted_foreground = self.muted_foreground;
        colors.accent = self.accent;
        colors.accent_foreground = self.accent_foreground;
        colors.destructive = self.destructive;
        colors.destructive_foreground = self.destructive_foreground;
        colors.border = self.border;
        colors.input = self.input;
        colors.ring = self.ring;
        colors.selection = self.selection;

        colors
    }
}

pub(crate) fn observe(cx: &mut App) {
    cx.observe_global::<Theme>(synchronize).detach();
    synchronize(cx);
}

fn synchronize(cx: &mut App) {
    #[cfg(not(all(feature = "kit", not(target_family = "wasm"))))]
    let semantic = Semantic::from_base(&Theme::global(cx));

    #[cfg(all(feature = "kit", not(target_family = "wasm")))]
    let semantic = {
        let kit = gpui_kit::component::Theme::global(cx);
        let mut base = Theme::default();

        base.appearance = if kit.mode.is_dark() {
            Appearance::Dark
        } else {
            Appearance::Light
        };
        base.tokens = kit.semantic_tokens();

        Semantic::from_base(&base)
    };

    if cx
        .try_global::<ThemeGlobal<Semantic>>()
        .is_none_or(|current| *current.0 != semantic)
    {
        super::replace(cx, semantic);
    }
}

pub(super) fn install(cx: &mut App, semantic: &Semantic) {
    crate::init(cx);
    #[cfg(not(all(feature = "kit", not(target_family = "wasm"))))]
    {
        let base = Theme::global_mut(cx);

        base.appearance = semantic.appearance;
        base.tokens.colors = semantic.colors();
    }
    #[cfg(all(feature = "kit", not(target_family = "wasm")))]
    {
        use gpui_kit::component::{Theme as KitTheme, ThemeMode};
        let mode = match semantic.appearance {
            Appearance::Light => ThemeMode::Light,
            Appearance::Dark => ThemeMode::Dark,
        };

        if KitTheme::global(cx).mode != mode {
            // Kit loads mode defaults over edits made in the same update.
            KitTheme::change(mode, None, cx);
        }

        let mut tokens = KitTheme::global(cx).semantic_tokens();

        tokens.colors = semantic.colors();
        KitTheme::update(cx, |theme| {
            theme.apply_semantic_tokens(&tokens);
            // Kit 0.9's apply_semantic_tokens omits this Base color role.
            theme.selection = semantic.selection;
        });
    }
    synchronize(cx);
}
