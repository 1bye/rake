//! Covers: policy opt-in, window identity, debounce, close/quit, and reset ordering.
//! Real: GPUI observers/scheduler and atomic preferences in isolated directories.
//! Doubles: GPUI TestPlatform windows and clock. Requires: no display server.

use super::*;
use crate::platform::Preferences;

fn policy() -> (tempfile::TempDir, WindowState) {
    let directory = tempfile::tempdir().expect("isolated preferences directory");
    let policy = WindowState::new(Preferences::at(directory.path()));

    policy.load();

    (directory, policy)
}

fn options() -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds_to_gpui(Rect {
            x: 50.,
            y: 50.,
            width: 600.,
            height: 400.,
        }))),
        ..Default::default()
    }
}

fn open(app: &gpui::TestAppContext, id: WindowId) -> gpui::AnyWindowHandle {
    app.update(|cx| {
        let mut options = options();
        let placement = prepare(id, &mut options, cx);

        cx.open_window(options, |window, cx| {
            observe(placement, window, cx);

            cx.new(|_| gpui::Empty)
        })
        .expect("test window opens")
        .into()
    })
}

fn finish_writes(app: &gpui::TestAppContext, policy: &WindowState) {
    app.executor().allow_parking();
    app.foreground_executor().block_test(async {
        let _write = policy.inner.writes.lock().await;
    });
}

#[test]
fn absent_policy_missing_record_and_opted_out_window_keep_defaults() {
    let app = gpui::TestAppContext::single();
    let (_directory, policy) = policy();

    app.update(|cx| {
        let mut options = options();
        let expected = options.window_bounds;

        assert!(prepare(WindowId(1), &mut options, cx).is_none());
        assert_eq!(options.window_bounds, expected);
        install(policy, cx);
        assert!(prepare(WindowId(1), &mut options, cx).is_some());
        assert!(prepare(WindowId(2), &mut options, cx).is_none());
        assert_eq!(options.window_bounds, expected);
    });
}

#[test]
fn stable_keys_are_independent_and_duplicate_live_keys_opt_out() {
    let app = gpui::TestAppContext::single();
    let (_directory, policy) = policy();
    let policy = policy.keys(|id| Some(if id == WindowId(2) { "editor" } else { "main" }.into()));

    app.update(|cx| install(policy.clone(), cx));
    let main = open(&app, WindowId(1));
    let editor = open(&app, WindowId(2));
    let duplicate = open(&app, WindowId(3));

    app.simulate_window_resize(main, gpui::size(gpui::px(700.), gpui::px(500.)));
    app.simulate_window_resize(editor, gpui::size(gpui::px(800.), gpui::px(600.)));
    app.simulate_window_resize(duplicate, gpui::size(gpui::px(900.), gpui::px(700.)));
    let records = policy.inner.records.borrow();

    assert_eq!(records["main"].bounds.width, 700.);
    assert_eq!(records["editor"].bounds.width, 800.);
    assert_eq!(records.len(), 2);
    app.update(|cx| assert_eq!(cx.global::<Driver>().windows.len(), 2));
}

#[test]
fn resize_burst_is_coalesced_until_interval_then_saves_latest_bounds() {
    let app = gpui::TestAppContext::single();
    let (directory, policy) = policy();

    app.update(|cx| install(policy.clone(), cx));
    let window = open(&app, WindowId(1));

    app.run_until_parked();
    app.simulate_window_resize(window, gpui::size(gpui::px(650.), gpui::px(450.)));
    app.simulate_window_resize(window, gpui::size(gpui::px(750.), gpui::px(550.)));
    app.executor()
        .advance_clock(SAVE_INTERVAL - Duration::from_millis(1));
    app.run_until_parked();
    assert!(!directory.path().join(super::super::PREFERENCE_KEY).exists());
    app.executor().advance_clock(Duration::from_millis(1));
    app.run_until_parked();
    finish_writes(&app, &policy);
    let data = std::fs::read_to_string(directory.path().join(super::super::PREFERENCE_KEY))
        .expect("coalesced write completed");

    assert_eq!(placement::decode(&data)["main"].bounds.width, 750.);
    app.update(|cx| assert!(!cx.global::<Driver>().signals.dirty.get()));
}

#[test]
fn close_before_worker_starts_flushes_without_waiting_for_debounce_and_key_reopens() {
    let app = gpui::TestAppContext::single();
    let (directory, policy) = policy();

    app.update(|cx| install(policy.clone(), cx));
    let window = open(&app, WindowId(1));

    app.simulate_window_resize(window, gpui::size(gpui::px(750.), gpui::px(550.)));
    app.update(|cx| {
        window
            .update(cx, |_, window, cx| {
                capture(window, cx);
                window.remove_window();
            })
            .expect("close live window")
    });
    app.run_until_parked();
    finish_writes(&app, &policy);
    let data = std::fs::read_to_string(directory.path().join(super::super::PREFERENCE_KEY))
        .expect("final close write completed");

    assert_eq!(placement::decode(&data)["main"].bounds.width, 750.);
    app.update(|cx| assert!(cx.global::<Driver>().windows.is_empty()));
    let reopened = open(&app, WindowId(1));

    app.update(|cx| {
        reopened
            .update(cx, |_, window, _| {
                assert_eq!(f32::from(window.bounds().size.width), 750.)
            })
            .expect("reopened window")
    });
}

#[test]
fn fullscreen_keeps_normal_bounds_and_quit_flushes_before_debounce() {
    let app = gpui::TestAppContext::single();
    let (directory, policy) = policy();

    app.executor().allow_parking();
    app.update(|cx| install(policy.clone(), cx));
    let window = open(&app, WindowId(1));

    app.simulate_window_resize(window, gpui::size(gpui::px(750.), gpui::px(550.)));
    app.update(|cx| {
        window
            .update(cx, |_, window, cx| {
                window.toggle_fullscreen();
                window.bounds_changed(cx);
            })
            .expect("fullscreen test window")
    });
    app.simulate_window_resize(window, gpui::size(gpui::px(1920.), gpui::px(1080.)));
    app.quit();
    let data = std::fs::read_to_string(directory.path().join(super::super::PREFERENCE_KEY))
        .expect("quit awaits final preference write");

    let records = placement::decode(&data);

    assert_eq!(records["main"].mode, Mode::Fullscreen);
    assert_eq!(records["main"].bounds.width, 750.);
}

#[test]
fn reset_serializes_with_queued_saves_and_preserves_other_windows() {
    let app = gpui::TestAppContext::single();
    let (directory, policy) = policy();
    let policy = policy.keys(|id| Some(if id == WindowId(2) { "editor" } else { "main" }.into()));

    app.executor().allow_parking();
    app.update(|cx| install(policy.clone(), cx));
    open(&app, WindowId(1));
    open(&app, WindowId(2));
    app.run_until_parked();
    app.foreground_executor()
        .block_test(policy.reset("main"))
        .expect("reset preference");
    app.executor().advance_clock(SAVE_INTERVAL);
    app.run_until_parked();
    finish_writes(&app, &policy);
    let data = std::fs::read_to_string(directory.path().join(super::super::PREFERENCE_KEY))
        .expect("preferences exist");

    let records = placement::decode(&data);

    assert!(!records.contains_key("main"));
    assert!(records.contains_key("editor"));
    let fresh = WindowState::new(Preferences::at(directory.path()));

    fresh.load();
    assert_eq!(*fresh.inner.records.borrow(), records);
}

#[test]
fn all_modes_restore_the_same_normal_bounds() {
    let app = gpui::TestAppContext::single();
    let (_directory, policy) = policy();

    app.update(|cx| {
        install(policy.clone(), cx);
        let display = cx.primary_display().expect("test display");

        let bounds = Rect {
            x: 50.,
            y: 50.,
            width: 600.,
            height: 400.,
        };

        for mode in [Mode::Windowed, Mode::Maximized, Mode::Fullscreen] {
            policy.inner.records.borrow_mut().insert(
                "main".into(),
                SavedWindow {
                    bounds,
                    display: display.uuid().ok().map(|uuid| uuid.to_string()),
                    mode,
                    decoration_height: 0.,
                },
            );
            let mut options = options();
            let prepared = prepare(WindowId(1), &mut options, cx).expect("mapped main window");

            let expected = match mode {
                Mode::Windowed => WindowBounds::Windowed(bounds_to_gpui(bounds)),
                Mode::Maximized => WindowBounds::Maximized(bounds_to_gpui(bounds)),
                Mode::Fullscreen => WindowBounds::Fullscreen(bounds_to_gpui(bounds)),
            };

            assert_eq!(options.window_bounds, Some(expected));
            assert_eq!(prepared.normal, Some(bounds));
        }
    });
}

#[test]
fn invalid_keys_opt_out_without_creating_preference_records() {
    let app = gpui::TestAppContext::single();
    let (_directory, policy) = policy();
    let policy = policy.keys(|_| Some(String::new()));

    app.update(|cx| install(policy.clone(), cx));
    open(&app, WindowId(1));

    assert!(policy.inner.records.borrow().is_empty());
    app.update(|cx| assert!(cx.global::<Driver>().windows.is_empty()));
}

#[test]
fn changed_scale_preserves_logical_bounds_and_stable_key_restores_with_new_runtime_id() {
    let app = gpui::TestAppContext::single();
    let (directory, policy) = policy();
    let policy = policy.keys(|_| Some("editor".into()));

    app.executor().allow_parking();
    app.update(|cx| install(policy.clone(), cx));
    let editor = open(&app, WindowId(2));

    app.simulate_window_resize(editor, gpui::size(gpui::px(750.), gpui::px(550.)));
    app.simulate_window_scale_factor_change(editor, 2.);
    assert_eq!(policy.inner.records.borrow()["editor"].bounds.width, 750.);
    app.update(|cx| {
        editor
            .update(cx, |_, window, cx| {
                capture(window, cx);
                window.remove_window();
            })
            .expect("close editor")
    });
    app.run_until_parked();
    finish_writes(&app, &policy);

    // A new TestPlatform generates different display UUIDs. Reuse the host so
    // this test keeps the monitor connected while replacing the policy/cache.
    let loaded = WindowState::new(Preferences::at(directory.path()))
        .keys(|id| (id == WindowId(42)).then(|| "editor".into()));

    loaded.load();
    app.update(|cx| install(loaded, cx));
    let reopened = open(&app, WindowId(42));

    app.update(|cx| {
        reopened
            .update(cx, |_, window, _| {
                assert_eq!(f32::from(window.bounds().size.width), 750.);
                assert_eq!(f32::from(window.bounds().size.height), 550.);
            })
            .expect("stable editor key restores despite a different runtime ID")
    });
}

#[test]
fn startup_zoom_frames_keep_restore_bounds_and_rejected_modes_eventually_follow_host() {
    let app = gpui::TestAppContext::single();
    let (_directory, policy) = policy();

    let bounds = Rect {
        x: 50.,
        y: 50.,
        width: 600.,
        height: 400.,
    };

    app.update(|cx| install(policy.clone(), cx));
    let window = app.update(|cx| {
        // TestPlatform's zoom method panics. Inject the prepared request at the
        // observer boundary while its physical test window stays windowed.
        cx.open_window(options(), |window, cx| {
            observe(
                Some(Prepared {
                    key: "main".into(),
                    normal: Some(bounds),
                    mode: Some(Mode::Maximized),
                }),
                window,
                cx,
            );

            cx.new(|_| gpui::Empty)
        })
        .expect("window for mode transition test")
        .into()
    });

    // TestPlatform does not implement zoom; these mimic windowed animation frames.
    app.simulate_window_resize(window, gpui::size(gpui::px(1000.), gpui::px(700.)));
    app.simulate_window_resize(window, gpui::size(gpui::px(1500.), gpui::px(900.)));
    assert_eq!(policy.inner.records.borrow()["main"].bounds, bounds);
    assert_eq!(policy.inner.records.borrow()["main"].mode, Mode::Maximized);
    app.run_until_parked();
    app.executor().advance_clock(Duration::from_secs(2));
    app.run_until_parked();

    assert_eq!(policy.inner.records.borrow()["main"].mode, Mode::Windowed);
    assert_eq!(policy.inner.records.borrow()["main"].bounds.width, 1500.);
}
