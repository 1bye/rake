//! Synchronous cleanup for retained native controls when an AppKit window closes.

use objc2::{MainThreadOnly as _, rc::Retained, sel};
use objc2_app_kit::{NSWindow, NSWindowWillCloseNotification};
use objc2_foundation::NSNotificationCenter;

use super::action::ActionTarget;

pub(super) struct CloseObserver {
    window: Retained<NSWindow>,
    center: Retained<NSNotificationCenter>,
    target: Retained<ActionTarget>,
}

impl CloseObserver {
    pub fn new(window: Retained<NSWindow>, callback: impl FnMut() + 'static) -> Self {
        let target = ActionTarget::new(window.mtm());

        target.set_callback(callback);
        let center = NSNotificationCenter::defaultCenter();

        // SAFETY: The retained main-thread target implements this one-object
        // selector and ignores its NSObject sender. This window's AppKit close
        // notification runs on the main thread; Drop unregisters before release.
        unsafe {
            center.addObserver_selector_name_object(
                &target,
                sel!(rakeNativeAction:),
                Some(NSWindowWillCloseNotification),
                Some(&window),
            );
        }

        Self {
            window,
            center,
            target,
        }
    }

    pub fn is_in(&self, window: &NSWindow) -> bool {
        std::ptr::eq(&*self.window, window)
    }
}

impl Drop for CloseObserver {
    fn drop(&mut self) {
        // SAFETY: This is the exact retained observer registered by new, still
        // alive on the main thread, and removed before its target is released.
        unsafe { self.center.removeObserver(&self.target) };
    }
}
