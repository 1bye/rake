//! Local AppKit adoption probe: actual deferred termination, decline and forwarding restoration.
#[cfg(target_os = "macos")]
fn main() {
    use std::ffi::c_void;
    unsafe extern "C" {
        fn rake_native_quit_test_app_create() -> *mut c_void;
        fn rake_native_quit_test_app_destroy(handle: *mut c_void);
        fn rake_native_quit_test_request();
        fn rake_native_quit_test_reply() -> std::ffi::c_long;
        fn rake_native_quit_test_forward() -> std::ffi::c_long;
        fn rake_native_quit_test_restored(handle: *mut c_void) -> bool;
    }
    struct App(*mut c_void);
    impl Drop for App {
        fn drop(&mut self) {
            // SAFETY: this unique test application delegate is released on the main thread.
            unsafe { rake_native_quit_test_app_destroy(self.0) };
        }
    }
    eprintln!("creating isolated AppKit application");
    // SAFETY: this main-thread test owns AppKit initialization and no window/data from another process.
    let app = App(unsafe { rake_native_quit_test_app_create() });
    assert!(!app.0.is_null());
    eprintln!("installing owned quit proxy");
    let bridge = rake_updates::NativeQuitBridge::new().unwrap();
    assert!(
        rake_updates::NativeQuitBridge::new().is_err(),
        "another owner must not steal the delegate"
    );
    // SAFETY: the bridge owns the main-thread delegate and forwards to the live test delegate.
    assert_eq!(unsafe { rake_native_quit_test_forward() }, 42);
    let changed = bridge.changed();
    eprintln!("requesting AppKit termination");
    // SAFETY: termination is deferred by our live bridge; this does not terminate a user application.
    unsafe { rake_native_quit_test_request() };
    eprintln!("termination request returned; waiting for notification");
    futures::executor::block_on(changed);
    assert!(bridge.take_request());
    assert!(!bridge.take_request());
    eprintln!("declining native termination");
    bridge.reply(false);
    bridge.prepare_exit(true);
    // SAFETY: asking the delegate directly observes approval without terminating this probe.
    assert_eq!(unsafe { rake_native_quit_test_reply() }, 1);
    bridge.prepare_exit(false);
    drop(bridge);
    // SAFETY: the test delegate remains retained by app until the end of main.
    assert!(unsafe { rake_native_quit_test_restored(app.0) });
    println!(
        "AppKit quit deferred, cancellation preserved the app, approval and delegate restoration passed"
    );
}
#[cfg(not(target_os = "macos"))]
fn main() {
    panic!("native quit adoption requires macOS");
}
