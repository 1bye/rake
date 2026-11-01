//! Keep live GPUI and native subtrees discoverable through AppKit's window.
//!
//! AccessKit replaces the content view's children, hiding native subviews.
//! Make that content view an accessible group and expose native host groups as
//! its siblings. Each group's children remain dynamic; no GPUI nodes are copied.

use std::{
    cell::RefCell,
    collections::BTreeMap,
    rc::{Rc, Weak},
};

use objc2::{rc::Retained, runtime::AnyObject};
use objc2_app_kit::{NSAccessibility, NSAccessibilityGroupRole, NSView, NSWindow};
use objc2_foundation::{NSArray, NSString};

thread_local! {
    static WINDOWS: RefCell<BTreeMap<usize, Weak<WindowTree>>> = const { RefCell::new(BTreeMap::new()) };
}

struct WindowTree {
    window: Retained<NSWindow>,
    content: Retained<NSView>,
    was_element: bool,
    role: Option<Retained<NSString>>,
    base_children: Option<Retained<NSArray>>,
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
        let base_children = window.accessibilityChildren();

        Some(Self {
            window,
            content,
            was_element,
            role,
            base_children,
            hosts: RefCell::new(BTreeMap::new()),
        })
    }

    fn update(&self) {
        let base: Vec<_> = self
            .base_children
            .as_ref()
            .map(|children| children.iter().collect())
            .unwrap_or_default();

        let hosts = self.hosts.borrow();
        let mut children: Vec<&AnyObject> = base.iter().map(|child| &**child).collect();

        for (_, view) in hosts.values() {
            children.push(view);
        }

        let children = NSArray::from_slice(&children);

        // SAFETY: All entries are existing AppKit accessibility elements or
        // native NSView groups; their retained owners outlive this copied array.
        unsafe { self.window.setAccessibilityChildren(Some(&children)) };
    }
}

impl Drop for WindowTree {
    fn drop(&mut self) {
        self.content.setAccessibilityElement(self.was_element);
        self.content.setAccessibilityRole(self.role.as_deref());
        // SAFETY: Clearing the explicit children restores AppKit's dynamic
        // traversal once the final native host has detached.
        unsafe { self.window.setAccessibilityChildren(None) };
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
        tree.update();

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
        self.tree.update();
    }
}
