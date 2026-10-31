//! Retained overlay ordering, focus ownership and frame reconciliation.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gpui::{
    AnyWeakEntity, App, Bounds, Context, FocusHandle, Pixels, Subscription, WeakFocusHandle,
    Window, WindowId,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Popover,
    Dialog,
}

pub(super) struct HostData {
    owner: AnyWeakEntity,
    window: Option<WindowId>,
    pub frame: u64,
    next_id: usize,
    pub root: FocusHandle,
    pub fallback: FocusHandle,
    stack: Vec<Weak<RefCell<OverlayData>>>,
}

/// Retained ordering and focus state for one window's overlay host.
#[derive(Clone)]
pub struct OverlayHostState(pub(super) Rc<RefCell<HostData>>);

impl OverlayHostState {
    /// Creates a host bound to this entity's lifetime and redraw notifications.
    pub fn new<T: 'static>(cx: &Context<T>) -> Self {
        Self(Rc::new(RefCell::new(HostData {
            owner: cx.entity().into_any().downgrade(),
            window: None,
            frame: 0,
            next_id: 0,
            root: cx.focus_handle(),
            fallback: cx.focus_handle(),
            stack: Vec::new(),
        })))
    }

    /// Creates a retained anchor to mount through [`super::OverlayAnchor`].
    pub fn anchor(&self) -> AnchorState {
        AnchorState {
            host: self.clone(),
            inner: Rc::new(RefCell::new(AnchorData {
                id: self.next_id(),
                frame: 0,
                bounds: None,
            })),
        }
    }

    /// Creates state for a [`super::Popover`], without constructing its content.
    pub fn popover(&self, cx: &App) -> OverlayState {
        self.create(Kind::Popover, cx)
    }

    /// Creates state for a [`super::Dialog`], without constructing its content.
    pub fn dialog(&self, cx: &App) -> OverlayState {
        self.create(Kind::Dialog, cx)
    }

    /// A mounted fallback focus target used when an overlay's opener disappears.
    pub fn focus_handle(&self) -> FocusHandle {
        self.0.borrow().fallback.clone()
    }

    /// Whether the stack includes a dialog and background interaction is modal.
    pub fn is_modal(&self) -> bool {
        self.entries()
            .iter()
            .any(|entry| entry.kind() == Kind::Dialog)
    }

    /// The number of currently open overlays.
    pub fn len(&self) -> usize {
        self.entries().len()
    }

    /// Whether the stack has no open overlays.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn next_id(&self) -> usize {
        let mut host = self.0.borrow_mut();
        let id = host.next_id;

        host.next_id = id.checked_add(1).expect("overlay handle IDs exhausted");

        id
    }

    fn create(&self, kind: Kind, cx: &App) -> OverlayState {
        OverlayState {
            host: self.clone(),
            inner: Rc::new(RefCell::new(OverlayData {
                id: self.next_id(),
                kind,
                open: false,
                frame: 0,
                opened_frame: 0,
                bounds: None,
                root: cx.focus_handle(),
                first: cx.focus_handle().tab_stop(true),
                last: cx.focus_handle().tab_stop(true),
                return_focus: None,
                needs_focus: false,
                anchor: None,
                subscriptions: Vec::new(),
            })),
        }
    }

    pub(super) fn bind(&self, window: &Window) {
        let mut host = self.0.borrow_mut();
        let id = window.window_handle().window_id();

        assert!(
            host.window.is_none_or(|bound| bound == id),
            "an overlay host belongs to one window"
        );
        host.window = Some(id);
    }

    pub(super) fn notify(&self, window: &mut Window, cx: &mut App) {
        let owner = &self.0.borrow().owner;

        if owner.is_upgradable() {
            cx.notify(owner.entity_id());
        }

        window.refresh();
    }

    pub(super) fn entries(&self) -> Vec<OverlayState> {
        let entries: Vec<_> = self
            .0
            .borrow()
            .stack
            .iter()
            .filter_map(Weak::upgrade)
            .collect();

        entries
            .into_iter()
            .filter(|entry| entry.borrow().open)
            .map(|inner| OverlayState {
                host: self.clone(),
                inner,
            })
            .collect()
    }

    pub(super) fn mounted(&self, window: &Window) -> bool {
        let host = self.0.borrow();

        host.root.contains(&host.fallback, window)
    }

    pub(super) fn begin_frame(&self, window: &Window) {
        self.bind(window);
        let mut host = self.0.borrow_mut();

        host.frame = host.frame.wrapping_add(1).max(1);
    }

    pub(super) fn audit(&self, window: &mut Window, cx: &mut App) {
        let frame = self.0.borrow().frame;

        for entry in self.entries() {
            let missing = {
                let state = entry.inner.borrow();

                state.opened_frame != frame
                    && (state.frame != frame
                        || state.anchor.as_ref().is_some_and(|anchor| {
                            let anchor = anchor.inner.borrow();

                            anchor.frame != frame || anchor.bounds.is_none()
                        }))
            };

            if missing {
                entry.close(window, cx);
            }
        }

        if let Some(top) = self.entries().last() {
            let needs_focus = top.inner.borrow().needs_focus;

            if needs_focus && top.inner.borrow().frame == frame {
                top.inner.borrow_mut().needs_focus = false;
                top.focus_first(window, cx);
            } else if !needs_focus
                && top.modal_context()
                && !top.focus_handle().contains_focused(window, cx)
            {
                top.focus_first(window, cx);
            }
        }
    }
}

pub(super) struct AnchorData {
    pub id: usize,
    pub frame: u64,
    pub bounds: Option<Bounds<Pixels>>,
}

/// Retained trigger geometry, updated by [`super::OverlayAnchor`] on every prepaint.
#[derive(Clone)]
pub struct AnchorState {
    pub(super) host: OverlayHostState,
    pub(super) inner: Rc<RefCell<AnchorData>>,
}

impl AnchorState {
    /// The most recently painted visible trigger bounds in window coordinates.
    pub fn bounds(&self) -> Option<Bounds<Pixels>> {
        self.inner.borrow().bounds
    }
}

pub(super) struct OverlayData {
    pub id: usize,
    pub kind: Kind,
    pub open: bool,
    pub frame: u64,
    opened_frame: u64,
    pub bounds: Option<Bounds<Pixels>>,
    pub root: FocusHandle,
    pub first: FocusHandle,
    pub last: FocusHandle,
    return_focus: Option<WeakFocusHandle>,
    needs_focus: bool,
    pub anchor: Option<AnchorState>,
    subscriptions: Vec<Subscription>,
}

/// A retained popover or dialog handle belonging to one [`OverlayHostState`].
#[derive(Clone)]
pub struct OverlayState {
    pub(super) host: OverlayHostState,
    pub(super) inner: Rc<RefCell<OverlayData>>,
}

impl OverlayState {
    /// Opens once, remembers the current focus and schedules focus inside the next frame.
    /// Returns false if already open or the owning entity has been released.
    ///
    /// # Panics
    /// Panics if used in a different window from its host.
    pub fn open(&self, window: &mut Window, cx: &mut App) -> bool {
        self.host.bind(window);

        if self.is_open() || !self.host.0.borrow().owner.is_upgradable() {
            return false;
        }

        {
            let mut state = self.inner.borrow_mut();

            // Input may open a layer after this frame was painted but before its audit.
            state.opened_frame = self.host.0.borrow().frame;
            state.open = true;
            state.needs_focus = true;
            state.return_focus = window.focused(cx).map(|focus| focus.downgrade());
        }

        self.host
            .0
            .borrow_mut()
            .stack
            .push(Rc::downgrade(&self.inner));
        self.install_focus_guards(window, cx);
        window.focus(&self.focus_handle(), cx);
        self.host.notify(window, cx);

        true
    }

    /// Closes this overlay and overlays opened above it. Restores a still-mounted
    /// opener when focus belonged to the removed stack; otherwise preserves focus.
    ///
    /// # Panics
    /// Panics if used in a different window from its host.
    pub fn close(&self, window: &mut Window, cx: &mut App) -> bool {
        self.host.bind(window);

        let entries = self.host.entries();

        let Some(index) = entries
            .iter()
            .position(|entry| Rc::ptr_eq(&entry.inner, &self.inner))
        else {
            return false;
        };

        let removed = &entries[index..];
        let root = self.host.0.borrow().root.clone();

        let restore = removed
            .iter()
            .any(|entry| entry.focus_handle().contains_focused(window, cx))
            || window
                .focused(cx)
                .is_none_or(|focus| !root.contains(&focus, window));

        let target = self
            .inner
            .borrow_mut()
            .return_focus
            .take()
            .and_then(|focus| focus.upgrade());

        self.host.0.borrow_mut().stack = entries[..index]
            .iter()
            .map(|entry| Rc::downgrade(&entry.inner))
            .collect();

        for entry in removed {
            let mut state = entry.inner.borrow_mut();

            state.open = false;
            state.needs_focus = false;
            state.bounds = None;
            state.subscriptions.clear();
            state.return_focus = None;
        }

        if restore {
            let target = target.filter(|focus| {
                root.contains(focus, window)
                    && !removed
                        .iter()
                        .any(|entry| entry.focus_handle().contains(focus, window))
            });

            if let Some(target) = target {
                window.focus(&target, cx);
            } else if let Some(top) = self.host.entries().last() {
                top.focus_first(window, cx);
            } else if self.host.mounted(window) {
                window.focus(&self.host.focus_handle(), cx);
            } else {
                window.blur();
            }
        }

        self.host.notify(window, cx);

        true
    }

    /// Whether this overlay is open, including when another overlay is above it.
    pub fn is_open(&self) -> bool {
        self.inner.borrow().open
    }

    /// Whether this overlay is the topmost open layer.
    pub fn is_topmost(&self) -> bool {
        self.host
            .entries()
            .last()
            .is_some_and(|top| Rc::ptr_eq(&top.inner, &self.inner))
    }

    /// The overlay container's retained focus handle.
    pub fn focus_handle(&self) -> FocusHandle {
        self.inner.borrow().root.clone()
    }

    /// The most recently rendered surface bounds, useful for positioning diagnostics.
    pub fn bounds(&self) -> Option<Bounds<Pixels>> {
        self.inner.borrow().bounds
    }

    pub(super) fn kind(&self) -> Kind {
        self.inner.borrow().kind
    }

    pub(super) fn modal_context(&self) -> bool {
        self.host
            .entries()
            .iter()
            .take_while(|entry| !Rc::ptr_eq(&entry.inner, &self.inner))
            .any(|entry| entry.kind() == Kind::Dialog)
            || self.kind() == Kind::Dialog
    }

    pub(super) fn priority(&self) -> usize {
        self.host
            .entries()
            .iter()
            .position(|entry| Rc::ptr_eq(&entry.inner, &self.inner))
            .unwrap_or(0)
            + 1
    }

    pub(super) fn focus_first(&self, window: &mut Window, cx: &mut App) {
        let state = self.inner.borrow();

        window.focus(&state.first, cx);
        window.focus_next(cx);

        if state.last.is_focused(window) || !state.root.contains_focused(window, cx) {
            window.focus(&state.root, cx);
        }
    }

    pub(super) fn navigate(&self, backwards: bool, window: &mut Window, cx: &mut App) {
        let state = self.inner.borrow();

        if backwards {
            if state.root.is_focused(window) {
                window.focus(&state.last, cx);
            }

            window.focus_prev(cx);
        } else {
            if state.root.is_focused(window) {
                window.focus(&state.first, cx);
            }

            window.focus_next(cx);
        }

        if self.modal_context() {
            if state.first.is_focused(window) {
                window.focus(&state.last, cx);
                window.focus_prev(cx);
            } else if state.last.is_focused(window) {
                window.focus(&state.first, cx);
                window.focus_next(cx);
            }

            if state.first.is_focused(window) || state.last.is_focused(window) {
                window.focus(&state.root, cx);
            }
        }
    }

    fn install_focus_guards(&self, window: &mut Window, cx: &mut App) {
        let (first, last, root) = {
            let state = self.inner.borrow();

            (state.first.clone(), state.last.clone(), state.root.clone())
        };

        let mut subscriptions = Vec::new();

        for (sentinel, backwards) in [(first, true), (last, false)] {
            let weak = self.downgrade();

            subscriptions.push(
                window.on_focus_in(&sentinel.clone(), cx, move |window, cx| {
                    if !sentinel.is_focused(window) {
                        return;
                    }

                    if let Some(state) = weak.upgrade().filter(Self::is_topmost) {
                        if state.modal_context() {
                            let data = state.inner.borrow();

                            if backwards {
                                window.focus(&data.last, cx);
                                window.focus_prev(cx);
                            } else {
                                window.focus(&data.first, cx);
                                window.focus_next(cx);
                            }

                            if data.first.is_focused(window) || data.last.is_focused(window) {
                                window.focus(&data.root, cx);
                            }
                        } else {
                            if backwards {
                                window.focus_prev(cx);
                            } else {
                                window.focus_next(cx);
                            }

                            state.close(window, cx);
                        }
                    }
                }),
            );
        }

        let weak = self.downgrade();

        subscriptions.push(window.on_focus_out(&root, cx, move |_, window, cx| {
            if let Some(state) = weak.upgrade().filter(Self::is_topmost) {
                if !state.host.mounted(window) {
                    state.close(window, cx);
                } else if !state.focus_handle().contains_focused(window, cx) {
                    if state.modal_context() {
                        state.focus_first(window, cx);
                    } else {
                        state.close(window, cx);
                    }
                }
            }
        }));
        self.inner.borrow_mut().subscriptions = subscriptions;
    }

    fn downgrade(&self) -> WeakOverlay {
        WeakOverlay {
            inner: Rc::downgrade(&self.inner),
            host: Rc::downgrade(&self.host.0),
        }
    }
}

struct WeakOverlay {
    inner: Weak<RefCell<OverlayData>>,
    host: Weak<RefCell<HostData>>,
}

impl WeakOverlay {
    fn upgrade(&self) -> Option<OverlayState> {
        Some(OverlayState {
            host: OverlayHostState(self.host.upgrade()?),
            inner: self.inner.upgrade()?,
        })
    }
}
