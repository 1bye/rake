//! Route window hit testing through native hosts before the GPUI canvas.

use std::{ffi::CString, mem, sync::Mutex};

use objc2::{
    encode::Encode,
    ffi, msg_send,
    runtime::{AnyClass, AnyObject, Imp, Sel},
    sel,
};

use objc2_app_kit::NSWindow;
use objc2_foundation::NSPoint;

use super::WINDOWS;

type HitTester = unsafe extern "C-unwind" fn(&NSWindow, Sel, NSPoint) -> *mut AnyObject;

static ORIGINALS: Mutex<Vec<(&'static AnyClass, Option<HitTester>)>> = Mutex::new(Vec::new());

fn window_class(window: &NSWindow) -> &'static AnyClass {
    // SAFETY: The retained window is live. Objective-C runtime classes have
    // process lifetime. NSObject::class hides KVO subclasses, where Base
    // installs its forwarder, so method interception needs object_getClass.
    unsafe { ffi::object_getClass(std::ptr::from_ref(window).cast()).as_ref() }
        .expect("live NSWindow runtime class")
}

pub(super) fn intercept(window: &NSWindow) {
    let mut originals = ORIGINALS.lock().expect("native hit testers lock poisoned");
    let mut ancestor = Some(window_class(window));

    while let Some(class) = ancestor {
        if originals.iter().any(|(installed, _)| *installed == class) {
            return;
        }

        ancestor = class.superclass();
    }

    let class = window_class(window);
    let selector = sel!(accessibilityHitTest:);

    // SAFETY: The selector takes a screen NSPoint and returns an autoreleased
    // accessibility object. Save the exact ABI, including Base's forwarder.
    let original = class
        .instance_method(selector)
        .map(|method| unsafe { mem::transmute::<_, HitTester>(method.implementation()) });

    let encoding = CString::new(format!("@@:{}", NSPoint::ENCODING))
        .expect("Objective-C point encoding has no NUL");

    originals.push((class, original));

    // SAFETY: Erase only the function pointer type; hit_test preserves the
    // receiver, selector, NSPoint argument and accessibility-object return ABI.
    let replacement: Imp = unsafe { mem::transmute(hit_test as HitTester) };

    // SAFETY: Replace only this runtime window class's method with the same ABI.
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

unsafe extern "C-unwind" fn hit_test(
    window: &NSWindow,
    selector: Sel,
    point: NSPoint,
) -> *mut AnyObject {
    let tree = WINDOWS
        .try_with(|windows| {
            windows
                .borrow()
                .get(&(std::ptr::from_ref(window) as usize))
                .and_then(std::rc::Weak::upgrade)
        })
        .ok()
        .flatten();

    if let Some(tree) = tree {
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

    let original = {
        let originals = ORIGINALS.lock().expect("native hit testers lock poisoned");
        let mut class = window_class(window);

        loop {
            if let Some((_, original)) = originals.iter().find(|(candidate, _)| *candidate == class)
            {
                break *original;
            }

            class = class.superclass().expect("intercepted window hit tester");
        }
    };

    if let Some(original) = original {
        // SAFETY: Preserve the saved window method's receiver, selector and ABI.
        unsafe { original(window, selector, point) }
    } else if let Some(content) = window.contentView() {
        // SAFETY: NSView implements the same screen-coordinate accessibility query.
        unsafe { msg_send![&*content, accessibilityHitTest: point] }
    } else {
        std::ptr::null_mut()
    }
}
