//! Covers: ambient theme lifecycle and observable GPUI global updates.
//! Real: theme runtime, App globals and subscriptions.
//! Doubles: GPUI TestPlatform; no native window or GPU required.

use std::cell::Cell;
use std::rc::Rc;

use rake::gpui;
use rake::theme::{self, ThemeGlobal};

#[test]
fn themes_coexist_replace_without_clone_and_notify_global_observers() {
    struct Colors(u32);

    struct Metrics(u32);

    let cx = gpui::TestAppContext::single();

    assert!(theme::try_current::<Colors>().is_none());
    cx.update(|cx| {
        theme::install(cx, Colors(1));
        theme::install(cx, Metrics(2));
    });
    let old = theme::current::<Colors>();
    let observed = Rc::new(Cell::new(0));
    let values = observed.clone();

    let _subscription = cx.update(|cx| {
        cx.observe_global::<ThemeGlobal<Colors>>(move |cx| {
            values.set(cx.global::<ThemeGlobal<Colors>>().0.0);
        })
    });

    cx.run_until_parked();
    cx.update(|cx| theme::install(cx, Colors(3)));
    cx.run_until_parked();

    assert_eq!(observed.get(), 3);
    assert_eq!(old.0, 1, "existing handles retain the previous theme");
    assert_eq!(theme::current::<Colors>().0, 3);
    assert_eq!(theme::current::<Metrics>().0, 2);
}

#[test]
fn missing_theme_panic_names_the_type_and_setup_fix() {
    #[derive(Debug)]
    struct NotInstalled;

    let panic = std::panic::catch_unwind(theme::current::<NotInstalled>).unwrap_err();
    let message = panic.downcast_ref::<String>().expect("formatted panic");

    assert!(message.contains("NotInstalled"));
    assert!(message.contains("rake::theme::install(cx, ...)"));
}
