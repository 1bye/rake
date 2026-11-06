//! Route window hit testing through native hosts before the GPUI canvas.
//!
//! Hooks persist on the stable class and any overriding KVO subclass for the
//! process lifetime. Hits outside native hosts forward to the saved method,
//! including Base's canvas forwarder, or to the content view when none existed.

use std::{ffi::CString, mem, sync::Mutex};

use objc2::{
    encode::Encode,
    ffi, msg_send,
    runtime::{AnyClass, AnyObject, Imp, Sel},
    sel,
};

use objc2_app_kit::NSWindow;
use objc2_foundation::NSPoint;

use super::{OriginalCall, WINDOWS};

type HitTester = unsafe extern "C-unwind" fn(&NSWindow, Sel, NSPoint) -> *mut AnyObject;

static ORIGINALS: Mutex<Vec<(&'static AnyClass, Option<HitTester>)>> = Mutex::new(Vec::new());

pub(super) fn intercept(window: &NSWindow) {
    let mut originals = ORIGINALS.lock().expect("native hit testers lock poisoned");
    let selector = sel!(accessibilityHitTest:);

    let encoding = CString::new(format!("@@:{}", NSPoint::ENCODING))
        .expect("Objective-C point encoding has no NUL");

    // SAFETY: Erase only the function pointer type; hit_test preserves the
    // receiver, selector, NSPoint argument and accessibility-object return ABI.
    let replacement: Imp = unsafe { mem::transmute(hit_test as HitTester) };

    // As with children, hook the stable class for KVO removal. Base uses
    // class_addMethod on the runtime subclass, so also replace that override
    // while preserving its canvas forwarder for points outside native hosts.
    for class in [super::window_class(window), window.class()] {
        if originals.iter().any(|(installed, _)| *installed == class) {
            continue;
        }

        let method = class.instance_method(selector);

        if method.is_some_and(|method| std::ptr::fn_addr_eq(method.implementation(), replacement)) {
            continue;
        }

        // SAFETY: Save the selector's exact screen-NSPoint/object-return ABI,
        // including Base's forwarder when present on this class.
        let original =
            method.map(|method| unsafe { mem::transmute::<_, HitTester>(method.implementation()) });

        originals.push((class, original));

        // SAFETY: Replace only the method, with the same ABI and class layout.
        // Registrations are main-thread owned; other windows retain their fallback.
        unsafe {
            ffi::class_replaceMethod(
                std::ptr::from_ref(class).cast_mut(),
                selector,
                replacement,
                encoding.as_ptr(),
            );
        }
    }
}

unsafe extern "C-unwind" fn hit_test(
    window: &NSWindow,
    selector: Sel,
    point: NSPoint,
) -> *mut AnyObject {
    let superclass = OriginalCall::superclass(window, selector);

    let tree = WINDOWS
        .try_with(|windows| {
            windows
                .borrow()
                .get(&(std::ptr::from_ref(window) as usize))
                .and_then(std::rc::Weak::upgrade)
        })
        .ok()
        .flatten();

    if let Some(tree) = tree.filter(|_| superclass.is_none()) {
        let in_window = window.convertPointFromScreen(point);
        // SAFETY: The tree retains this live main-thread content view.
        let parent = unsafe { tree.content.superview() };

        let in_parent = parent.as_ref().map_or(in_window, |parent| {
            parent.convertPoint_fromView(in_window, None)
        });

        // AppKit resolves stacking, clipping and the actual native descendant.
        // Native hosts can sit beneath GPUI's inner canvas, not directly under
        // NSWindow.contentView, so identify them along the hit view's ancestry.
        if let Some(hit) = tree.content.hitTest(in_parent) {
            let mut ancestor = Some(hit.clone());

            while let Some(view) = ancestor {
                let is_host = tree
                    .hosts
                    .borrow()
                    .values()
                    .any(|(_, host)| std::ptr::eq(&**host, &*view));

                if is_host {
                    // SAFETY: The live hit view belongs to one of our retained
                    // native hosts. AppKit owns its accessibility representation.
                    return unsafe { msg_send![&*hit, accessibilityHitTest: point] };
                }

                // SAFETY: Hierarchy access stays on AppKit's main thread.
                ancestor = unsafe { view.superview() };
            }
        }
    }

    let (class, original) = {
        let originals = ORIGINALS.lock().expect("native hit testers lock poisoned");
        let mut class = superclass.unwrap_or_else(|| window.class());

        loop {
            if let Some((_, original)) = originals.iter().find(|(candidate, _)| *candidate == class)
            {
                break (class, *original);
            }

            class = class.superclass().expect("intercepted window hit tester");
        }
    };

    if let Some(original) = original {
        let _call = OriginalCall::new(window, selector, class);

        // SAFETY: Preserve the saved window method's receiver, selector and ABI.
        unsafe { original(window, selector, point) }
    } else if let Some(content) = window.contentView() {
        // SAFETY: NSView implements the same screen-coordinate accessibility query.
        unsafe { msg_send![&*content, accessibilityHitTest: point] }
    } else {
        std::ptr::null_mut()
    }
}
