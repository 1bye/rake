//! Covers: public Semantic installation and real Base-global observation without loops.
//! Real: GPUI global effects, Base theme projection and Rake ambient themes.
//! Doubles: TestPlatform; no display or GPU.

use rake_ui::theme::{self, Appearance, Semantic, ThemeGlobal};
use std::{cell::Cell, rc::Rc};

#[cfg(not(feature = "kit"))]
#[test]
fn semantic_install_and_base_edits_share_one_observed_palette() {
    let cx = &mut gpui::TestAppContext::single();

    struct AppTheme {
        label: &'static str,
    }
    cx.update(|cx| {
        rake_ui::init(cx);
        theme::install(
            cx,
            AppTheme {
                label: "application-owned",
            },
        );
        theme::install(cx, Semantic::dark());
        assert_eq!(*theme::current::<Semantic>(), Semantic::dark());
        assert_eq!(
            gpui_base::Theme::global(cx).tokens.colors.surface,
            Semantic::dark().surface
        );
        assert_eq!(gpui_base::Theme::global(cx).appearance, Appearance::Dark);
    });
    cx.run_until_parked();
    let changes = Rc::new(Cell::new(0));

    let observer = cx.update(|cx| {
        let changes = changes.clone();

        cx.observe_global::<ThemeGlobal<Semantic>>(move |_| changes.set(changes.get() + 1))
    });

    let primary = gpui::Hsla::from(gpui::rgb(0x123456));

    cx.update(|cx| gpui_base::Theme::global_mut(cx).tokens.colors.primary = primary);
    cx.run_until_parked();
    assert_eq!(
        theme::current::<Semantic>().primary,
        primary,
        "the Base observer must actually fire"
    );
    assert_eq!(changes.get(), 1);
    assert_eq!(theme::current::<AppTheme>().label, "application-owned");
    cx.update(|cx| {
        let _ = gpui_base::Theme::global_mut(cx);
    });
    cx.run_until_parked();
    assert_eq!(
        changes.get(),
        1,
        "unchanged globals must not replace the ambient theme or loop"
    );
    drop(observer);
}

#[cfg(feature = "kit")]
#[test]
fn kit_updates_trigger_the_base_observer_and_keep_semantic_colors_in_sync() {
    let cx = &mut gpui::TestAppContext::single();

    use rake_ui::kit::component::Theme;
    let mut palette = Semantic::dark();

    palette.surface = gpui::rgb(0x112233).into();
    palette.destructive = gpui::rgb(0xcc2211).into();
    palette.selection = gpui::rgb(0x3355ff).into();
    cx.update(|cx| {
        theme::install(cx, palette);
        assert_eq!(Theme::global(cx).popover, palette.surface);
        assert_eq!(Theme::global(cx).danger, palette.destructive);
        assert_eq!(Theme::global(cx).selection, palette.selection);
        assert_eq!(
            gpui_base::Theme::global(cx).tokens,
            Theme::global(cx).semantic_tokens()
        );
    });
    cx.run_until_parked();
    assert_eq!(*theme::current::<Semantic>(), palette);
    let changes = Rc::new(Cell::new(0));

    let observer = cx.update(|cx| {
        let changes = changes.clone();

        cx.observe_global::<ThemeGlobal<Semantic>>(move |_| changes.set(changes.get() + 1))
    });

    let primary = gpui::Hsla::from(gpui::rgb(0x123456));

    cx.update(|cx| Theme::update(cx, |theme| theme.primary = primary));
    cx.run_until_parked();
    assert_eq!(
        theme::current::<Semantic>().primary,
        primary,
        "Kit's Base projection must trigger Rake's observer"
    );
    assert_eq!(changes.get(), 1);
    cx.update(|cx| Theme::update(cx, |_| {}));
    cx.run_until_parked();
    assert_eq!(changes.get(), 1, "unchanged Kit projections must not loop");
    cx.update(|cx| Theme::change(rake_ui::kit::component::ThemeMode::Light, None, cx));
    cx.run_until_parked();
    assert_eq!(theme::current::<Semantic>().appearance, Appearance::Light);
    cx.update(|cx| {
        assert_eq!(
            theme::current::<Semantic>().surface,
            Theme::global(cx).semantic_tokens().colors.surface
        )
    });
    drop(observer);
}

#[cfg(feature = "kit")]
#[test]
fn initializing_rake_preserves_a_hosts_existing_kit_theme() {
    use rake_ui::kit::component::{Theme, ThemeMode};
    let cx = &mut gpui::TestAppContext::single();
    let primary = gpui::Hsla::from(gpui::rgb(0x123456));

    cx.update(|cx| {
        rake_ui::kit::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        Theme::update(cx, |theme| {
            theme.primary = primary;
            theme.radius = gpui::px(17.);
        });
        rake_ui::init(cx);
        assert!(
            Theme::global(cx).mode.is_dark(),
            "Rake must preserve the host's Kit appearance"
        );
        assert_eq!(
            Theme::global(cx).primary,
            primary,
            "Rake must preserve the host's Kit colors"
        );
        assert_eq!(theme::current::<Semantic>().primary, primary);
        assert_eq!(Theme::global(cx).radius, gpui::px(17.));
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(Theme::global(cx).mode.is_dark());
        assert_eq!(Theme::global(cx).primary, primary);
        assert_eq!(theme::current::<Semantic>().primary, primary);
    });
}
