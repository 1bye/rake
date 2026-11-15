//! Covers: live appearance, forced preferences, palette replacement and the semantic bridge.
//! Real: GPUI observer queue and Rake theme installation.
//! Doubles: GPUI TestPlatform, without a display or GPU.

#[cfg(feature = "gpui-fast")]
extern crate gpui_fast as gpui;
#[cfg(not(feature = "gpui-fast"))]
extern crate gpui_pre as gpui;

use rake_ui::theme::{self, Semantic, ThemeMode};

#[test]
fn appearance_updates_and_forced_modes() {
    let cx = &mut gpui::TestAppContext::single();
    cx.update(|cx| {
        rake_ui::init(cx);
        assert_eq!(theme::mode(cx), None);
        assert!(!theme::set_mode(cx, ThemeMode::Dark));
        theme::register(cx, ThemeMode::System, Semantic::light(), Semantic::dark());
    });
    let window = cx.add_window(|window, cx| {
        theme::observe_window(window, cx);
        gpui::Empty
    });
    let platform_window = cx.test_window(window.into());
    cx.update(|_| platform_window.simulate_appearance_change(gpui::WindowAppearance::Dark));
    cx.run_until_parked();
    assert_eq!(*theme::current::<Semantic>(), Semantic::dark());
    cx.update(|cx| {
        assert_eq!(theme::mode(cx), Some(ThemeMode::System));
        assert!(theme::set_mode(cx, ThemeMode::Light));
    });
    cx.run_until_parked();
    assert_eq!(*theme::current::<Semantic>(), Semantic::light());
    cx.update(|_| platform_window.simulate_appearance_change(gpui::WindowAppearance::Light));
    cx.run_until_parked();
    cx.update(|_| platform_window.simulate_appearance_change(gpui::WindowAppearance::Dark));
    cx.run_until_parked();
    assert_eq!(*theme::current::<Semantic>(), Semantic::light());
    cx.update(|cx| {
        assert!(theme::set_mode(cx, ThemeMode::System));
    });
    cx.run_until_parked();
    assert_eq!(*theme::current::<Semantic>(), Semantic::dark());
    cx.update(|cx| theme::register(cx, ThemeMode::Dark, 1_u32, 2_u32));
    cx.update(|_| platform_window.simulate_appearance_change(gpui::WindowAppearance::Light));
    cx.run_until_parked();
    assert_eq!(*theme::current::<u32>(), 2);
    cx.update(|cx| {
        assert!(theme::set_mode(cx, ThemeMode::System));
    });
    assert_eq!(*theme::current::<u32>(), 1);
}
