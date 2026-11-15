//! Application appearance preference and live window observation.

use std::rc::Rc;

use gpui::{App, Global, Window, WindowAppearance};

/// Appearance preference suitable for saving in application preferences.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeMode {
    /// Follow the operating system or browser preference.
    #[default]
    System,
    /// Always use the registered light palette.
    Light,
    /// Always use the registered dark palette.
    Dark,
}

struct Selection {
    mode: ThemeMode,
    appearance: WindowAppearance,
    apply: Rc<dyn Fn(bool, &mut App)>,
}

impl Global for Selection {}

/// Registers a palette pair and applies the selected appearance immediately.
/// Later calls replace the pair; existing window observers use the new pair.
pub fn register<T: Clone + 'static>(cx: &mut App, mode: ThemeMode, light: T, dark: T) {
    let appearance = cx.window_appearance();
    cx.set_global(Selection {
        mode,
        appearance,
        apply: Rc::new(move |is_dark, cx| {
            super::install(cx, if is_dark { dark.clone() } else { light.clone() })
        }),
    });
    apply(cx);
}

/// Returns the registered preference, or `None` before palette registration.
pub fn mode(cx: &App) -> Option<ThemeMode> {
    cx.try_global::<Selection>().map(|selection| selection.mode)
}

/// Changes the preference without replacing the registered palettes.
/// Returns false if no palette pair has been registered.
pub fn set_mode(cx: &mut App, mode: ThemeMode) -> bool {
    if !cx.has_global::<Selection>() {
        return false;
    }

    let selection = cx.global_mut::<Selection>();
    selection.mode = mode;
    apply(cx);

    true
}

fn apply(cx: &mut App) {
    let selection = cx.global::<Selection>();
    let is_dark = match selection.mode {
        ThemeMode::System => matches!(
            selection.appearance,
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        ),
        ThemeMode::Light => false,
        ThemeMode::Dark => true,
    };
    let apply = selection.apply.clone();
    apply(is_dark, cx);
}

/// Connects a custom window to the application's appearance preference.
/// Rake-managed windows call this automatically, before mounting views.
pub fn observe_window(window: &mut Window, cx: &mut App) {
    update(window, cx);
    window.observe_window_appearance(update).detach();
}

fn update(window: &mut Window, cx: &mut App) {
    if !cx.has_global::<Selection>() {
        return;
    }

    let selection = cx.global_mut::<Selection>();
    selection.appearance = window.appearance();

    if selection.mode == ThemeMode::System {
        apply(cx);
    }
}
