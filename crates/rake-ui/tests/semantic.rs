//! Covers: public Semantic installation and real Base-global observation without loops.
//! Real: GPUI global effects, Base theme projection and Rake ambient themes.
//! Doubles: TestPlatform; no display or GPU.

use rake_ui::theme::{self, Appearance, Semantic, ThemeGlobal};
use std::{cell::Cell, rc::Rc};

#[gpui::test]
fn semantic_install_and_base_edits_share_one_observed_palette(cx: &mut gpui::TestAppContext) {
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
