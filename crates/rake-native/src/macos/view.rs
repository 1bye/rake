//! Mount native views above GPUI and release their attachment with element state.

use std::{
    cell::Cell,
    rc::Rc,
    sync::atomic::{AtomicUsize, Ordering},
};

use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    Pixels, Refineable, Style, StyleRefinement, Styled, Visibility, Window, px, size,
};

use objc2::{MainThreadMarker, rc::Retained};
use objc2_app_kit::{NSAccessibility, NSControl, NSView};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::{NativeError, accessibility::Registration, action::ActionTarget};

static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

/// A retained AppKit view shared by its owner and its mounted GPUI element.
///
/// Clones refer to the same view. Mount it in one layout slot at a time. Keep
/// it in your entity instead of constructing it on every render. Removing the
/// element or closing its window detaches it even if this handle survives.
#[derive(Clone)]
pub struct NativeView {
    inner: Rc<ViewInner>,
}

struct ViewInner {
    container: Retained<NSView>,
    view: Retained<NSView>,
    measure: Retained<NSView>,
    // AppKit's control target is not retained by the control itself.
    action: Option<(Retained<NSControl>, Retained<ActionTarget>)>,
    id: usize,
    owner: Cell<usize>,
}

impl NativeView {
    /// Takes a retained AppKit view and makes it available to GPUI layout.
    ///
    /// Use the re-exported [`super::appkit`] and [`super::objc2`] bindings.
    /// The host owns attachment; do not also reparent the view outside Rake.
    /// Intrinsic AppKit dimensions supply the default GPUI size.
    ///
    /// # Errors
    /// Returns [`NativeError::NotMainThread`] outside the main thread.
    pub fn from_appkit(view: Retained<NSView>) -> Result<Self, NativeError> {
        Self::new(view.clone(), view, None)
    }

    pub(super) fn new(
        view: Retained<NSView>,
        measure: Retained<NSView>,
        action: Option<(Retained<NSControl>, Retained<ActionTarget>)>,
    ) -> Result<Self, NativeError> {
        let mtm = MainThreadMarker::new().ok_or(NativeError::NotMainThread)?;
        let container = NSView::new(mtm);

        container.setWantsLayer(true);

        if let Some(layer) = container.layer() {
            layer.setMasksToBounds(true);
        }

        container.addSubview(&view);

        Ok(Self {
            inner: Rc::new(ViewInner {
                container,
                view,
                measure,
                action,
                id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
                owner: Cell::new(0),
            }),
        })
    }

    /// Creates a layout element. Width and height override intrinsic sizing.
    ///
    /// Rectangular scroll clipping and the element's own visibility and opacity
    /// are applied to the native host. GPUI paint order and ancestor opacity
    /// groups do not extend to native views.
    pub fn element(&self) -> NativeElement {
        NativeElement {
            view: self.clone(),
            style: StyleRefinement::default(),
        }
    }
}

impl std::fmt::Debug for NativeView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeView")
            .field("id", &self.inner.id)
            .finish_non_exhaustive()
    }
}

/// GPUI layout slot for a [`NativeView`], created with [`NativeView::element`].
#[derive(Debug)]
pub struct NativeElement {
    view: NativeView,
    style: StyleRefinement,
}

impl Styled for NativeElement {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl IntoElement for NativeElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for NativeElement {
    type RequestLayoutState = Style;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(ElementId::named_usize("rake-native", self.view.inner.id))
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        _: &mut App,
    ) -> (LayoutId, Style) {
        let mut style = Style::default();

        style.refine(&self.style);
        let intrinsic = self.view.inner.measure.fittingSize();

        let layout = window.request_measured_layout(style.clone(), move |known, _, _, _| {
            size(
                known.width.unwrap_or(px(intrinsic.width.max(0.0) as f32)),
                known.height.unwrap_or(px(intrinsic.height.max(0.0) as f32)),
            )
        });

        (layout, style)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Style,
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        style: &mut Style,
        _: &mut (),
        window: &mut Window,
        _: &mut App,
    ) {
        let view = self.view.inner.clone();
        let clip = bounds.intersect(&window.content_mask().bounds);

        window.with_element_state(
            id.expect("native elements have stable IDs"),
            |state: Option<Attachment>, window| {
                let mut attachment = state.unwrap_or_else(|| Attachment::new(view));

                if let Some(host) = host_view(window) {
                    attachment.place(&host, bounds, clip, style);
                } else {
                    attachment.detach();
                }

                ((), attachment)
            },
        );
    }
}

fn host_view(window: &Window) -> Option<Retained<NSView>> {
    let handle = HasWindowHandle::window_handle(window).ok()?;

    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return None;
    };

    // SAFETY: GPUI's borrowed window handle provides a live NSView on the
    // main thread. Retaining it keeps it alive during this placement operation.
    unsafe { Retained::retain(handle.ns_view.as_ptr().cast::<NSView>()) }
}

struct Attachment {
    inner: Rc<ViewInner>,
    token: usize,
    accessibility: Option<Registration>,
}

impl Attachment {
    fn new(inner: Rc<ViewInner>) -> Self {
        Self {
            inner,
            token: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            accessibility: None,
        }
    }

    fn place(
        &mut self,
        host: &NSView,
        bounds: Bounds<Pixels>,
        clip: Bounds<Pixels>,
        style: &Style,
    ) {
        self.inner.owner.set(self.token);
        let container = &self.inner.container;
        // SAFETY: All hierarchy access uses retained main-thread AppKit views.
        let parent = unsafe { container.superview() };

        if !parent
            .as_ref()
            .is_some_and(|parent| std::ptr::eq(&**parent, host))
        {
            container.removeFromSuperview();
            host.addSubview(container);
        }

        if let Some(window) = host.window()
            && !self
                .accessibility
                .as_ref()
                .is_some_and(|tree| tree.is_in(&window))
        {
            self.accessibility = None;
            self.accessibility = Registration::new(window, container, self.inner.id, self.token);
        }

        let frame = native_frame(host, clip);

        container.setFrame(frame);
        let full = native_frame(host, bounds);

        self.inner
            .view
            .setFrame(host.convertRect_toView(full, Some(container)));
        container.setAlphaValue(style.opacity.unwrap_or(1.0).clamp(0.0, 1.0) as f64);
        let hidden = style.visibility != Visibility::Visible
            || clip.size.width <= px(0.)
            || clip.size.height <= px(0.);

        container.setHidden(hidden);
        container.setAccessibilityHidden(hidden || container.alphaValue() == 0.0);
    }

    fn detach(&mut self) {
        self.accessibility = None;

        if self.inner.owner.get() == self.token {
            self.inner.container.removeFromSuperview();
            self.inner.owner.set(0);
        }
    }
}

impl Drop for Attachment {
    fn drop(&mut self) {
        self.detach();
    }
}

fn native_frame(host: &NSView, bounds: Bounds<Pixels>) -> NSRect {
    let host_bounds = host.bounds();
    let x = bounds.origin.x.as_f32() as f64;
    let y = bounds.origin.y.as_f32() as f64;
    let width = bounds.size.width.as_f32() as f64;
    let height = bounds.size.height.as_f32() as f64;

    NSRect::new(
        NSPoint::new(
            host_bounds.origin.x + x,
            host_bounds.origin.y
                + if host.isFlipped() {
                    y
                } else {
                    host_bounds.size.height - y - height
                },
        ),
        NSSize::new(width, height),
    )
}

impl Drop for ViewInner {
    fn drop(&mut self) {
        self.container.removeFromSuperview();

        if let Some((control, _)) = &self.action {
            // SAFETY: Clearing the action before releasing our target also
            // protects a native control retained externally via as_appkit().
            unsafe {
                control.setTarget(None);
                control.setAction(None);
            }
        }
    }
}
