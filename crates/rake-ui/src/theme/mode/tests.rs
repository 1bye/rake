//! Covers: live appearance, forced preferences, palette replacement and the semantic bridge.
//! Real: appearance callback handler, GPUI observer queue and Rake theme installation.
//! Doubles: appearance events (GPUI test_window is private), no display or GPU.

#[cfg(feature = "gpui-fast")]
extern crate gpui_fast as gpui;
#[cfg(not(feature = "gpui-fast"))]
extern crate gpui_pre as gpui;

use crate::theme::{self, Semantic, ThemeMode};

#[test]
fn appearance_updates_and_forced_modes() {
    let cx = &mut gpui::TestAppContext::single();
    cx.update(|cx| {
        crate::init(cx);
        assert_eq!(theme::mode(cx), None);
        assert!(!theme::set_mode(cx, ThemeMode::Dark));
        theme::register(cx, ThemeMode::System, Semantic::light(), Semantic::dark());
    });
    cx.update(|cx| super::update_appearance(gpui::WindowAppearance::Dark, cx));
    cx.run_until_parked();
    assert_eq!(*theme::current::<Semantic>(), Semantic::dark());
    cx.update(|cx| {
        assert_eq!(
            gpui_base::Theme::global(cx).appearance,
            theme::Appearance::Dark
        );
        #[cfg(all(feature = "kit", not(target_family = "wasm")))]
        assert_eq!(
            crate::kit::component::Theme::global(cx).mode,
            crate::kit::component::ThemeMode::Dark
        );
    });
    cx.update(|cx| {
        assert_eq!(theme::mode(cx), Some(ThemeMode::System));
        assert!(theme::set_mode(cx, ThemeMode::Light));
    });
    cx.run_until_parked();
    assert_eq!(*theme::current::<Semantic>(), Semantic::light());
    cx.update(|cx| super::update_appearance(gpui::WindowAppearance::Light, cx));
    cx.run_until_parked();
    cx.update(|cx| super::update_appearance(gpui::WindowAppearance::Dark, cx));
    cx.run_until_parked();
    assert_eq!(*theme::current::<Semantic>(), Semantic::light());
    cx.update(|cx| {
        assert!(theme::set_mode(cx, ThemeMode::System));
    });
    cx.run_until_parked();
    assert_eq!(*theme::current::<Semantic>(), Semantic::dark());
    cx.update(|cx| theme::register(cx, ThemeMode::Dark, 1_u32, 2_u32));
    cx.update(|cx| super::update_appearance(gpui::WindowAppearance::Light, cx));
    cx.run_until_parked();
    assert_eq!(*theme::current::<u32>(), 2);
    cx.update(|cx| {
        assert!(theme::set_mode(cx, ThemeMode::System));
    });
    assert_eq!(*theme::current::<u32>(), 1);
}
