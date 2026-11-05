//! Experimental desktop GPU presentation through GPUI Fast composition.
//!
//! Retain one host per layout slot. The producer supplies its own device and
//! frame loop and observes the host for geometry and lifecycle changes.

use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    Pixels, Refineable, Style, StyleRefinement, Styled, Visibility, Window,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::atomic::{AtomicUsize, Ordering},
};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos::Host;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod native;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use native::Host;

/// The native attachment a renderer can present to.
///
/// Create the renderer using the variant available on the current platform.
/// Rake does not depend on the producer's graphics API or device.
#[non_exhaustive]
pub enum SurfaceTarget {
    /// An owned reference to the Metal layer. Keep it until the renderer releases its surface.
    #[cfg(target_os = "macos")]
    Metal(metal::MetalLayer),
    /// The GPUI-owned Linux child window or subsurface and its display connection.
    #[cfg(target_os = "linux")]
    Linux {
        /// XCB or Wayland window handle for the child surface.
        window: raw_window_handle::RawWindowHandle,
        /// Display handle from the same GPUI window.
        display: raw_window_handle::RawDisplayHandle,
    },
    /// An owned reference to the current DirectComposition visual.
    #[cfg(target_os = "windows")]
    DirectComposition(windows::Win32::Graphics::DirectComposition::IDCompositionVisual),
}

/// Latest physical extent, logical-to-physical scale and lifecycle status.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SurfaceState {
    /// Producer width in physical pixels. Linux and Windows use the clipped extent.
    pub width_px: u32,
    /// Producer height in physical pixels. Linux and Windows use the clipped extent.
    pub height_px: u32,
    /// Physical pixels per GPUI logical pixel.
    pub scale: f32,
    /// False while hidden, clipped out, removed or closed.
    pub visible: bool,
    /// Terminal state: stop presenting; a new window needs a new host.
    pub closed: bool,
}

/// Platform host initialization, target lookup or placement failure.
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
    changes: SurfaceChanges,
    host: Host,
    closed_subscription: RefCell<Option<gpui::Subscription>>,
    visibility_subscription: RefCell<Option<gpui::Subscription>>,
}

#[derive(Default)]
struct SurfaceChanges {
    state: Cell<SurfaceState>,
    handler: RefCell<Option<ChangeHandler>>,
}

impl SurfaceChanges {
    fn publish(&self, state: SurfaceState) {
        if self.state.replace(state) == state {
            return;
        }

        self.notify(state);
    }

    fn notify(&self, state: SurfaceState) {
        let Some(mut handler) = self.handler.borrow_mut().take() else {
            return;
        };
        handler(state);

        let mut slot = self.handler.borrow_mut();
        if slot.is_none() {
            *slot = Some(handler);
        }
    }

    fn on_change(&self, mut handler: impl FnMut(SurfaceState) + 'static) {
        handler(self.state.get());
        self.handler.replace(Some(Box::new(handler)));
    }
}

impl Inner {
    fn hide(&self) {
        self.host.hide();
        self.changes.publish(SurfaceState {
            visible: false,
            ..self.changes.state.get()
        });
    }
}

impl GpuSurface {
    /// Allocates a host in an existing window; no renderer or device is created.
    ///
    /// # Errors
    /// Returns a host initialization failure if GPUI cannot attach a native surface.
    pub fn new(window: &mut Window, cx: &mut App) -> Result<Self, SurfaceError> {
        let inner = Rc::new(Inner {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            changes: SurfaceChanges {
                state: Cell::new(SurfaceState {
                    scale: window.scale_factor(),
                    ..Default::default()
                }),
                ..Default::default()
            },
            host: Host::new(window, cx)?,
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
                inner.changes.publish(SurfaceState {
                    closed: true,
                    visible: false,
                    ..inner.changes.state.get()
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

        #[cfg(target_os = "windows")]
        {
            let weak = Rc::downgrade(&inner);
            let cx = cx.to_async();
            inner.host.on_compositor_recreated(move || {
                let weak = weak.clone();
                // Fast invokes recovery while its renderer is borrowed.
                cx.spawn(async move |_| {
                    if let Some(inner) = weak.upgrade() {
                        let state = inner.changes.state.get();
                        if !state.closed {
                            inner.changes.notify(state);
                        }
                    }
                })
                .detach();
            })?;
        }

        Ok(Self { inner })
    }

    /// Returns the current renderer target.
    ///
    /// On Linux, the raw handles borrow the platform window and connection.
    /// A producer must release its graphics surface before the host or window
    /// is destroyed. The raw handles do not keep either alive.
    ///
    /// On Windows, query the target again after a composition recovery
    /// notification and recreate the swapchain if the visual has changed.
    /// Keep the visual alive until the associated graphics surface is released.
    /// Call [`Self::commit_target`] after attaching or replacing the swapchain.
    ///
    /// # Errors
    /// Returns an error if the window has closed or GPUI cannot supply the target.
    pub fn target(&self) -> Result<SurfaceTarget, SurfaceError> {
        if self.state().closed {
            return Err(SurfaceError("the host window has closed".into()));
        }
        #[cfg(target_os = "macos")]
        return Ok(SurfaceTarget::Metal(self.metal_layer()));
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        self.inner.host.target()
    }

    /// Applies a renderer's target attachment changes to the window compositor.
    ///
    /// Call on the UI thread with the owning window after configuring a
    /// swapchain on the target. Windows commits the DirectComposition tree.
    /// On macOS and Linux the producer's normal presentation applies the changes.
    ///
    /// # Errors
    /// Returns an error if the host has closed or Windows cannot commit the tree.
    pub fn commit_target(&self, window: &Window) -> Result<(), SurfaceError> {
        if self.state().closed {
            return Err(SurfaceError("the host window has closed".into()));
        }
        #[cfg(target_os = "windows")]
        self.inner.host.commit_target(window)?;
        #[cfg(not(target_os = "windows"))]
        let _ = window;
        Ok(())
    }

    /// Retains the CAMetalLayer so a producer can configure and present on its thread.
    #[cfg(target_os = "macos")]
    pub fn metal_layer(&self) -> metal::MetalLayer {
        self.inner.host.metal_layer()
    }

    /// Returns the latest extent and visibility; initially extent is zero and hidden.
    pub fn state(&self) -> SurfaceState {
        self.inner.changes.state.get()
    }

    /// Installs one UI-thread callback, immediately delivering the latest state.
    /// A handler can replace itself during a state change.
    /// On Windows a compositor recovery can repeat the current state to signal
    /// that the producer needs to check for a replacement target.
    /// Native producers should forward this snapshot through a channel.
    pub fn on_change(&self, handler: impl FnMut(SurfaceState) + 'static) {
        self.inner.changes.on_change(handler);
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
        if inner.changes.state.get().closed {
            return;
        }
        let clip = bounds.intersect(&window.content_mask().bounds);
        let scale = window.scale_factor();
        let visible =
            style.visibility == Visibility::Visible && !clip.is_empty() && window.is_visible();
        // Fast's native child surfaces use one rectangle for size and clip.
        #[cfg(target_os = "macos")]
        let extent = bounds.size;
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        let extent = clip.size;
        let state = SurfaceState {
            width_px: (extent.width.as_f32() * scale).round().max(0.) as u32,
            height_px: (extent.height.as_f32() * scale).round().max(0.) as u32,
            scale,
            visible,
            closed: false,
        };
        if let Err(error) = inner.host.place(bounds, clip, state) {
            tracing::error!(%error, "GPU surface placement failed");
            inner.hide();
            return;
        }
        inner.changes.publish(state);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_handler_can_replace_itself_during_notification() {
        let changes = Rc::new(SurfaceChanges::default());
        let received = Rc::new(RefCell::new(Vec::new()));
        let first = Rc::clone(&received);
        let weak_changes = Rc::downgrade(&changes);

        changes.on_change(move |state| {
            first.borrow_mut().push(("first", state));
            if state.visible {
                let replacement = Rc::clone(&first);
                weak_changes.upgrade().unwrap().on_change(move |state| {
                    replacement.borrow_mut().push(("replacement", state));
                });
            }
        });

        let initial = SurfaceState::default();
        let visible = SurfaceState {
            visible: true,
            ..initial
        };
        let closed = SurfaceState {
            closed: true,
            ..initial
        };
        changes.publish(visible);
        changes.publish(visible);
        changes.publish(initial);
        changes.publish(closed);

        assert_eq!(
            *received.borrow(),
            [
                ("first", initial),
                ("first", visible),
                ("replacement", visible),
                ("replacement", initial),
                ("replacement", closed),
            ]
        );
    }
}
