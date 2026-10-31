//! Covers: information-only availability, cancellation, signed-feed errors and guard attachment.
//! Real: Sparkle, AppKit and a signed appcast served over loopback.
//! Doubles: none. Requires: macOS graphical login and the native probe script.
#[cfg(target_os = "macos")]
fn main() {
    use futures::FutureExt as _;
    use rake_updates::{NativeQuitBridge, UpdateController, UpdateService, UpdateState};
    use std::{
        ffi::c_void,
        rc::Rc,
        time::{Duration, Instant},
    };
    unsafe extern "C" {
        fn rake_native_quit_test_app_create() -> *mut c_void;
        fn rake_native_quit_test_app_destroy(handle: *mut c_void);
        fn rake_native_quit_test_window_count() -> usize;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFRunLoopDefaultMode: *const c_void;
        fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after_source: bool) -> i32;
    }
    struct App(*mut c_void);
    impl Drop for App {
        fn drop(&mut self) {
            // SAFETY: this main-thread test owns the retained delegate and outlives the quit guard.
            unsafe { rake_native_quit_test_app_destroy(self.0) };
        }
    }
    // SAFETY: this isolated bundled test initializes AppKit on its process main thread.
    let app = App(unsafe { rake_native_quit_test_app_create() });
    let resources = rake_resources::resources!().unwrap();
    let feed = std::fs::read_to_string(resources.path("feed-url.txt").unwrap()).unwrap();
    let service = UpdateService::new(
        option_env!("RAKE_UPDATE_POLICY").expect("run inside rake bundle"),
        &resources,
    )
    .unwrap()
    .testing_sparkle_feed(feed.trim())
    .unwrap();
    let updates = UpdateController::new(service);
    updates.initialize_sparkle_checks().unwrap();
    assert!(
        !updates.can_install(),
        "a probe must not grant installation authority"
    );
    let mode = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "available".into());
    let revision = updates.snapshot().revision;
    updates.check().unwrap();
    assert_eq!(updates.state(), UpdateState::Checking);
    assert!(futures::executor::block_on(updates.changed(revision)).revision > revision);
    assert!(matches!(updates.check(), Err(rake_updates::Error::Busy)));
    if mode == "cancel" {
        updates.cancel();
        assert_eq!(updates.state(), UpdateState::Cancelling);
    }
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        let revision = updates.snapshot().revision;
        let _ = updates.changed(revision).now_or_never();
        if !matches!(
            updates.state(),
            UpdateState::Checking | UpdateState::Cancelling
        ) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Sparkle probe timed out: {:?}",
            updates.state()
        );
        // SAFETY: the default-mode constant is provided by CoreFoundation; main-thread callbacks only.
        unsafe {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.02, false);
        }
    }
    match mode.as_str() {
        "available" => {
            let UpdateState::Available(info) = updates.state() else {
                panic!("{:?}", updates.state());
            };
            assert_eq!(info.version, "1.0.1");
            assert!(
                updates.download().is_err(),
                "a probe cannot start an installation flow"
            );
            assert!(matches!(updates.state(), UpdateState::Available(_)));
        }
        "current" => assert_eq!(updates.state(), UpdateState::UpToDate),
        "cancel" => assert_eq!(updates.state(), UpdateState::Idle),
        "invalid" => assert!(
            matches!(updates.state(), UpdateState::Failed { .. }),
            "{:?}",
            updates.state()
        ),
        _ => panic!("unknown native probe mode"),
    }
    let revision = updates.snapshot().revision;
    assert!(
        updates.changed(revision).now_or_never().is_none(),
        "a settled native state must not spin"
    );
    // SAFETY: AppKit is initialized and remains owned on this thread.
    assert_eq!(
        unsafe { rake_native_quit_test_window_count() },
        0,
        "information checks must not open native update UI"
    );
    let observed_state = updates.state();
    let quit = Rc::new(NativeQuitBridge::new().unwrap());
    updates.attach_sparkle(Rc::clone(&quit)).unwrap();
    assert!(updates.can_install());
    println!(
        "Sparkle shared controller {mode} passed: {:?}",
        observed_state
    );
    drop(updates);
    drop(quit);
    drop(app);
}
#[cfg(not(target_os = "macos"))]
fn main() {
    panic!("Sparkle acceptance requires macOS");
}
