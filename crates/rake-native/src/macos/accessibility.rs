//! Keep live GPUI and native subtrees discoverable through AppKit's window.
//!
//! AccessKit replaces the content view's children, hiding native subviews.
//! Make that content view an accessible group and expose native host groups as
//! its siblings. Compose children on each window accessibility query so content
//! and window chrome stay live; forward to AppKit when no hosts remain.

use std::{
    cell::RefCell,
    collections::BTreeMap,
    mem,
    rc::{Rc, Weak},
    sync::Mutex,
};

use objc2::{
    ffi,
    rc::Retained,
    runtime::{AnyClass, AnyObject, MethodImplementation, Sel},
    sel,
};

use objc2_app_kit::{NSAccessibility, NSAccessibilityGroupRole, NSView, NSWindow};
use objc2_foundation::{NSArray, NSString};

thread_local! {
    static WINDOWS: RefCell<BTreeMap<usize, Weak<WindowTree>>> = const { RefCell::new(BTreeMap::new()) };
}

type ChildrenGetter = unsafe extern "C-unwind" fn(&NSWindow, Sel) -> *mut NSArray;

// Runtime methods outlive registrations and thread-local window state.
static GETTERS: Mutex<Vec<(&'static AnyClass, ChildrenGetter)>> = Mutex::new(Vec::new());

fn intercept_children(window: &NSWindow) {
    let mut getters = GETTERS.lock().expect("native window getters lock poisoned");
    let mut class = Some(window.class());

    while let Some(current) = class {
        if getters.iter().any(|(class, _)| *class == current) {
            return;
        }

        class = current.superclass();
    }

    let class = window.class();
    let selector = sel!(accessibilityChildren);

    let method = class
        .instance_method(selector)
        .expect("NSWindow accessibility getter");

    // SAFETY: NSAccessibility's getter takes a receiver and selector and
    // returns an autoreleased NSArray. Preserve its exact ABI for forwarding.
    let original: ChildrenGetter = unsafe { mem::transmute(method.implementation()) };

    getters.push((class, original));

    // SAFETY: Install the same getter ABI on this window class only, without
    // changing its superclass or layout (including AppKit's KVO subclass).
    // Unregistered windows forward to the original implementation.
    unsafe {
        ffi::class_replaceMethod(
            std::ptr::from_ref(class).cast_mut(),
            selector,
            (window_children as unsafe extern "C-unwind" fn(_, _) -> _).__imp(),
            ffi::method_getTypeEncoding(method),
        );
    }
}

unsafe extern "C-unwind" fn window_children(window: &NSWindow, selector: Sel) -> *mut NSArray {
    let original = {
        let getters = GETTERS.lock().expect("native window getters lock poisoned");
        let mut class = window.class();

        loop {
            if let Some((_, original)) = getters.iter().find(|(candidate, _)| *candidate == class) {
                break *original;
            }

            class = class
                .superclass()
                .expect("getter belongs to an intercepted window class");
        }
    };

    let key = std::ptr::from_ref(window) as usize;

    let tree = WINDOWS
        .try_with(|windows| windows.borrow().get(&key).and_then(Weak::upgrade))
        .ok()
        .flatten();

    let Some(tree) = tree else {
        // SAFETY: Forward unchanged using the saved getter's original ABI.
        return unsafe { original(window, selector) };
    };

    // SAFETY: Call the saved getter with its original receiver, selector and
    // ABI, then retain its nullable autoreleased result during composition.
    let base = unsafe { Retained::retain(original(window, selector)) };

    let mut children: Vec<Retained<AnyObject>> = base
        .as_ref()
        .map(|children| children.iter().collect())
        .unwrap_or_default();

    for (_, view) in tree.hosts.borrow().values() {
        // A shared view may move before the old window drops its Attachment.
        if view
            .window()
            .is_some_and(|parent| std::ptr::eq(&*parent, window))
        {
            children.push(view.clone().into());
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
        let mut hosts = self.tree.hosts.borrow_mut();

        if hosts
            .get(&self.id)
            .is_some_and(|(token, _)| *token == self.token)
        {
            let (_, container) = hosts
                .remove(&self.id)
                .expect("registered native host exists");

            if container.accessibilityParent().is_some_and(|parent| {
                Retained::as_ptr(&parent) == Retained::as_ptr(&self.tree.window).cast()
            }) {
                // SAFETY: Clear only our parent: a newer attachment may already
                // have moved this shared native view into another window.
                unsafe { container.setAccessibilityParent(None) };
            }
        }

        drop(hosts);
    }
}
