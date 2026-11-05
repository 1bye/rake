//! Experimental macOS GPU presentation through GPUI Fast composition.
//!
//! Retain one host per layout slot. The Metal layer keeps its platform allocation
//! alive independently of the window. Stop the producer when `closed` is reported.
//! The producer supplies its own device and frame loop.

use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    Pixels, Refineable, Style, StyleRefinement, Styled, Visibility, Window,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::atomic::{AtomicUsize, Ordering},
};

mod macos;

/// Latest physical extent, logical-to-physical scale and lifecycle status.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SurfaceState {
    /// Full producer viewport in physical pixels, before rectangular clipping.
    pub width_px: u32,
    /// Full producer viewport in physical pixels, before rectangular clipping.
    pub height_px: u32,
    /// Physical pixels per GPUI logical pixel.
    pub scale: f32,
    /// False while hidden, clipped out, removed or closed.
    pub visible: bool,
    /// Terminal state: stop presenting; a new window needs a new host.
    pub closed: bool,
}

/// AppKit host initialization or placement failure.
#[derive(Debug, thiserror::Error)]
#[error("GPU surface host failed: {0}")]
pub struct SurfaceError(String);

static NEXT_ID: AtomicUsize = AtomicUsize::new(1);
type ChangeHandler = Box<dyn FnMut(SurfaceState)>;

/// A UI-thread host whose producer can own an independent device and frame loop.
#[derive(Clone)]
pub struct GpuSurface {
    inner: Rc<Inner>,
}

struct Inner {
    id: usize,
    state: Cell<SurfaceState>,
    change: RefCell<Option<ChangeHandler>>,
    host: macos::Host,
    closed_subscription: RefCell<Option<gpui::Subscription>>,
    visibility_subscription: RefCell<Option<gpui::Subscription>>,
}

impl Inner {
    fn publish(&self, state: SurfaceState) {
        if self.state.replace(state) != state
            && let Some(handler) = self.change.borrow_mut().as_mut()
        {
            handler(state);
        }
    }

    fn hide(&self) {
        self.host.hide();
        self.publish(SurfaceState {
            visible: false,
            ..self.state.get()
        });
    }
}

impl GpuSurface {
    /// Allocates a host in an existing window; no renderer or device is created.
    ///
    /// # Errors
    /// Returns a host initialization failure if AppKit or GPUI cannot attach the layer.
    pub fn new(window: &mut Window, cx: &mut App) -> Result<Self, SurfaceError> {
        let inner = Rc::new(Inner {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            state: Cell::new(SurfaceState {
                scale: window.scale_factor(),
                ..Default::default()
            }),
            change: RefCell::new(None),
            host: macos::Host::new(window, cx)?,
            closed_subscription: RefCell::new(None),
            visibility_subscription: RefCell::new(None),
        });
        let weak = Rc::downgrade(&inner);
        let window_id = window.window_handle().window_id();
        *inner.closed_subscription.borrow_mut() = Some(cx.on_window_closed(move |_, id| {
            if id == window_id
                && let Some(inner) = weak.upgrade()
            {
                inner.hide();
                inner.publish(SurfaceState {
                    closed: true,
                    visible: false,
                    ..inner.state.get()
                });
            }
        }));
        let weak = Rc::downgrade(&inner);
        *inner.visibility_subscription.borrow_mut() = Some(window.observe_window_visibility(
            move |visibility, window, _| {
                if visibility.is_visible() {
                    window.refresh();
                } else if let Some(inner) = weak.upgrade() {
                    inner.hide();
                }
            },
        ));

        Ok(Self { inner })
    }

    /// Retains the CAMetalLayer so a producer can configure and present on its thread.
    pub fn metal_layer(&self) -> metal::MetalLayer {
        self.inner.host.metal_layer()
    }

    /// Returns the latest extent and visibility; initially extent is zero and hidden.
    pub fn state(&self) -> SurfaceState {
        self.inner.state.get()
    }

    /// Installs one UI-thread callback, immediately delivering the latest state.
    /// Native producers should forward this snapshot through a channel.
    pub fn on_change(&self, mut handler: impl FnMut(SurfaceState) + 'static) {
        handler(self.state());
        *self.inner.change.borrow_mut() = Some(Box::new(handler));
    }

    /// Creates the single layout slot; element-state destruction hides the host on removal.
    pub fn element(&self) -> GpuSurfaceElement {
        GpuSurfaceElement {
            surface: self.clone(),
            style: StyleRefinement::default(),
        }
    }
}

/// Styled layout slot for a retained [`GpuSurface`].
pub struct GpuSurfaceElement {
    surface: GpuSurface,
    style: StyleRefinement,
}

impl IntoElement for GpuSurfaceElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Styled for GpuSurfaceElement {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

struct Mount(Rc<Inner>);
impl Drop for Mount {
    fn drop(&mut self) {
        self.0.hide();
    }
}

impl Element for GpuSurfaceElement {
    type RequestLayoutState = Style;
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        Some(ElementId::named_usize(
            "rake-gpu-surface",
            self.surface.inner.id,
        ))
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Style) {
        let mut style = Style::default();
        style.refine(&self.style);
        (window.request_layout(style.clone(), [], cx), style)
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        style: &mut Style,
        window: &mut Window,
        _: &mut App,
    ) {
        let inner = &self.surface.inner;
        if inner.state.get().closed {
            return;
        }
        let clip = bounds.intersect(&window.content_mask().bounds);
        let scale = window.scale_factor();
        let visible =
            style.visibility == Visibility::Visible && !clip.is_empty() && window.is_visible();
        let state = SurfaceState {
            width_px: (bounds.size.width.as_f32() * scale).round().max(0.) as u32,
            height_px: (bounds.size.height.as_f32() * scale).round().max(0.) as u32,
            scale,
            visible,
            closed: false,
        };
        if let Err(error) = inner.host.place(bounds, clip, state) {
            tracing::error!(%error, "GPU surface placement failed");
            inner.hide();
            return;
        }
        inner.publish(state);
    }
    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Style,
        _: &mut (),
        window: &mut Window,
        _: &mut App,
    ) {
        window.with_element_state(
            id.expect("GPU surface has a stable element ID"),
            |mount: Option<Mount>, _| {
                (
                    (),
                    mount.unwrap_or_else(|| Mount(self.surface.inner.clone())),
                )
            },
        );
    }
}
