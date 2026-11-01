//! Retained target/action bridge. User callbacks run later on GPUI's executor.

use std::cell::RefCell;

use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained};
use objc2_foundation::{NSObject, NSObjectProtocol};

pub(super) struct ActionIvars {
    pub callback: RefCell<Box<dyn FnMut()>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing obligations. All state and methods
    // stay on the main thread, and this class does not implement Drop.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "RakeNativeActionTarget"]
    #[ivars = ActionIvars]
    pub(super) struct ActionTarget;

    // SAFETY: NSObjectProtocol introduces no additional invariants.
    unsafe impl NSObjectProtocol for ActionTarget {}

    impl ActionTarget {
        // SAFETY: The selector has AppKit's target/action signature, with an
        // ignored object sender. It invokes only the internal scheduling closure.
        #[unsafe(method(rakeNativeAction:))]
        fn invoke(&self, _sender: &NSObject) {
            (self.ivars().callback.borrow_mut())();
        }
    }
);

impl ActionTarget {
    pub(super) fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let target = Self::alloc(mtm).set_ivars(ActionIvars {
            callback: RefCell::new(Box::new(|| {})),
        });

        // SAFETY: target is an allocated NSObject subclass with initialized ivars.
        unsafe { msg_send![super(target), init] }
    }

    pub(super) fn set_callback(&self, callback: impl FnMut() + 'static) {
        *self.ivars().callback.borrow_mut() = Box::new(callback);
    }
}
