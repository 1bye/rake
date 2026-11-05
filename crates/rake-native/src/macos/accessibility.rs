//! Keep live GPUI and native subtrees discoverable through AppKit's window.
//!
//! AccessKit replaces the content view's children, hiding native subviews.
//! Make that content view an accessible group and expose native host groups as
//! its siblings. Replace accessibilityChildren and accessibilityHitTest: on the
//! stable window class and overriding KVO subclasses for the life of the process.
//! Compose children on each query so content and window chrome stay live. When
//! the final host detaches, restore the content view's accessibility state and
//! forward window queries to their saved implementations.

mod hit_test;

use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    mem,
    rc::{Rc, Weak},
    sync::Mutex,
};

use objc2::{
    ffi, msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, Imp, Sel},
    sel,
};

use objc2_app_kit::{NSAccessibility, NSAccessibilityGroupRole, NSView, NSWindow};
use objc2_foundation::{NSArray, NSString};

thread_local! {
    static WINDOWS: RefCell<BTreeMap<usize, Weak<WindowTree>>> = const { RefCell::new(BTreeMap::new()) };
    // No Drop state: original forwarding remains available during thread-local teardown.
    static ORIGINAL_CALL: Cell<Option<(usize, Sel, &'static AnyClass)>> = const { Cell::new(None) };
}

// An original subclass method can send super to another intercepted class.
// Resume below that original and compose native hosts only at the outer call.
struct OriginalCall(Option<(usize, Sel, &'static AnyClass)>);

impl OriginalCall {
    fn superclass(window: &NSWindow, selector: Sel) -> Option<&'static AnyClass> {
        let key = std::ptr::from_ref(window) as usize;

        ORIGINAL_CALL
            .get()
            .filter(|(receiver, method, _)| *receiver == key && *method == selector)
            .map(|(_, _, class)| {
                class
                    .superclass()
                    .expect("window original has a superclass")
            })
    }

    fn new(window: &NSWindow, selector: Sel, class: &'static AnyClass) -> Self {
        Self(ORIGINAL_CALL.replace(Some((std::ptr::from_ref(window) as usize, selector, class))))
    }
}

impl Drop for OriginalCall {
    fn drop(&mut self) {
        ORIGINAL_CALL.set(self.0);
    }
}

type ChildrenGetter = unsafe extern "C-unwind" fn(&NSWindow, Sel) -> *mut NSArray;

// Runtime methods outlive registrations and thread-local window state.
static GETTERS: Mutex<Vec<(&'static AnyClass, ChildrenGetter)>> = Mutex::new(Vec::new());

fn window_class(window: &NSWindow) -> &'static AnyClass {
    // SAFETY: NSObject's class message returns the stable class hidden by KVO.
    unsafe { msg_send![window, class] }
}

fn intercept_children(window: &NSWindow) {
    let mut getters = GETTERS.lock().expect("native window getters lock poisoned");
    let selector = sel!(accessibilityChildren);
    // SAFETY: Erase only the function pointer type for Objective-C registration;
    // window_children has the getter's receiver, selector and NSArray return ABI.
    let replacement: Imp = unsafe { mem::transmute(window_children as ChildrenGetter) };

    // Keep the stable class hooked after KVO removal, and cover any override on
    // the current subclass without saving our inherited replacement as a fallback.
    for class in [window_class(window), window.class()] {
        if getters.iter().any(|(installed, _)| *installed == class) {
            continue;
        }

        let method = class
            .instance_method(selector)
            .expect("NSWindow accessibility getter");

        if std::ptr::fn_addr_eq(method.implementation(), replacement) {
            continue;
        }

        // SAFETY: NSAccessibility's getter takes a receiver and selector and
        // returns an autoreleased NSArray. Preserve its exact ABI for forwarding.
        let original: ChildrenGetter = unsafe { mem::transmute(method.implementation()) };

        getters.push((class, original));

        // SAFETY: Install the same getter ABI without changing class layout.
        // Unregistered windows forward to the original implementation.
        unsafe {
            ffi::class_replaceMethod(
                std::ptr::from_ref(class).cast_mut(),
                selector,
                replacement,
                ffi::method_getTypeEncoding(method),
            );
        }
    }
}

unsafe extern "C-unwind" fn window_children(window: &NSWindow, selector: Sel) -> *mut NSArray {
    let superclass = OriginalCall::superclass(window, selector);

    let (class, original) = {
        let getters = GETTERS.lock().expect("native window getters lock poisoned");
        let mut class = superclass.unwrap_or_else(|| window.class());

        loop {
            if let Some((_, original)) = getters.iter().find(|(candidate, _)| *candidate == class) {
                break (class, *original);
            }

            class = class
                .superclass()
                .expect("getter belongs to an intercepted window class");
        }
    };

    let base = {
        let _call = OriginalCall::new(window, selector, class);

        // SAFETY: Forward with the original receiver, selector and getter ABI.
        unsafe { original(window, selector) }
    };

    if superclass.is_some() {
        return base;
    }

    let key = std::ptr::from_ref(window) as usize;

    let tree = WINDOWS
        .try_with(|windows| windows.borrow().get(&key).and_then(Weak::upgrade))
        .ok()
        .flatten();

    let Some(tree) = tree else {
        return base;
    };

    // SAFETY: Retain the original getter's nullable autoreleased result.
    let base = unsafe { Retained::retain(base) };

    let mut children: Vec<Retained<AnyObject>> = base
        .as_ref()
        .map(|children| children.iter().collect())
        .unwrap_or_default();

    let hosts: Vec<_> = tree
        .hosts
        .borrow()
        .values()
        .map(|(_, view)| view.clone())
        .collect();

    for view in hosts {
        // A shared view may move before the old window drops its Attachment.
        if view
            .window()
            .is_some_and(|parent| std::ptr::eq(&*parent, window))
        {
            children.push(view.into());
        }
    }

    Retained::autorelease_return(NSArray::from_retained_slice(&children))
}

struct WindowTree {
    window: Retained<NSWindow>,
    content: Retained<NSView>,
    was_element: bool,
    role: Option<Retained<NSString>>,
    hosts: RefCell<BTreeMap<usize, (usize, Retained<NSView>)>>,
}

impl WindowTree {
    fn new(window: Retained<NSWindow>) -> Option<Self> {
        let content = window.contentView()?;
        let was_element = content.isAccessibilityElement();
        let role = content.accessibilityRole();

        content.setAccessibilityElement(true);
        // SAFETY: This is AppKit's standard accessibility group role.
        content.setAccessibilityRole(Some(unsafe { NSAccessibilityGroupRole }));
        intercept_children(&window);
        hit_test::intercept(&window);

        Some(Self {
            window,
            content,
            was_element,
            role,
            hosts: RefCell::new(BTreeMap::new()),
        })
    }
}

impl Drop for WindowTree {
    fn drop(&mut self) {
        self.content.setAccessibilityElement(self.was_element);
        self.content.setAccessibilityRole(self.role.as_deref());
    }
}

pub(super) struct Registration {
    tree: Rc<WindowTree>,
    id: usize,
    token: usize,
}

impl Registration {
    pub(super) fn new(
        window: Retained<NSWindow>,
        container: &NSView,
        id: usize,
        token: usize,
    ) -> Option<Self> {
        let key = Retained::as_ptr(&window) as usize;
        let existing = WINDOWS.with(|windows| windows.borrow().get(&key).and_then(Weak::upgrade));

        let tree = match existing {
            Some(tree) => tree,
            None => {
                let tree = Rc::new(WindowTree::new(window)?);

                WINDOWS.with(|windows| {
                    let mut windows = windows.borrow_mut();

                    windows.retain(|_, tree| tree.strong_count() > 0);
                    windows.insert(key, Rc::downgrade(&tree));
                });

                tree
            }
        };

        container.setAccessibilityElement(true);
        // SAFETY: This is AppKit's standard group role and its live parent window.
        unsafe {
            container.setAccessibilityRole(Some(NSAccessibilityGroupRole));
            container.setAccessibilityParent(Some(&tree.window));
        }
        // SAFETY: container is a live main-thread NSView retained by its host.
        let retained = unsafe { Retained::retain(std::ptr::from_ref(container).cast_mut()) }?;

        tree.hosts.borrow_mut().insert(id, (token, retained));

        Some(Self { tree, id, token })
    }

    pub(super) fn is_in(&self, window: &NSWindow) -> bool {
        std::ptr::eq(&*self.tree.window, window)
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        let container = {
            let mut hosts = self.tree.hosts.borrow_mut();

            if hosts
                .get(&self.id)
                .is_none_or(|(token, _)| *token != self.token)
            {
                return;
            }

            let (_, container) = hosts
                .remove(&self.id)
                .expect("registered native host exists");

            container
        };

        if container.accessibilityParent().is_some_and(|parent| {
            Retained::as_ptr(&parent) == Retained::as_ptr(&self.tree.window).cast()
        }) {
            // SAFETY: Clear only our parent: a newer attachment may already
            // have moved this shared native view into another window.
            unsafe { container.setAccessibilityParent(None) };
        }
    }
}
