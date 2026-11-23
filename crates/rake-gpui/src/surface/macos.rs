//! Main-thread AppKit attachment; only its retained CAMetalLayer crosses threads.

use foreign_types::ForeignType as _;
use gpui::{
    AnyWindowHandle, App, AsyncApp, Bounds, DevicePixels, PlatformSurfaceAttachment, Point, Window,
    WindowCompositionSurface,
};
use objc2::{MainThreadMarker, rc::Retained};
use objc2_app_kit::NSView;
use objc2_foundation::{NSPoint, NSRect, NSSize};
use std::{any::Any, cell::Cell, rc::Rc};

use super::{SurfaceError, SurfaceState};

pub(super) struct Host {
    surface: WindowCompositionSurface,
    attachment: Rc<Attachment>,
    window: AnyWindowHandle,
    cx: AsyncApp,
}

struct Attachment {
    container: Retained<NSView>,
    content: Retained<NSView>,
    layer: metal::MetalLayer,
    bounds: Cell<Bounds<DevicePixels>>,
}

impl Host {
    pub(super) fn new(window: &mut Window, cx: &mut App) -> Result<Self, SurfaceError> {
        let mtm = MainThreadMarker::new()
            .ok_or_else(|| SurfaceError("not on AppKit main thread".into()))?;
        let container = NSView::new(mtm);
        let content = NSView::new(mtm);
        container.setWantsLayer(true);
        content.setWantsLayer(true);
        let layer = metal::MetalLayer::new();
        layer.set_presents_with_transaction(false);
        // SAFETY: MetalLayer is a retained CAMetalLayer (a CALayer subclass).
        // Both views and all layer attachment mutations stay on AppKit's main thread.
        unsafe {
            let calayer = &*layer.as_ptr().cast::<objc2_quartz_core::CALayer>();
            content.setLayer(Some(calayer));
        }
        if let Some(layer) = container.layer() {
            layer.setMasksToBounds(true);
        }
        container.addSubview(&content);
        container.setHidden(true);
        let attachment = Rc::new(Attachment {
            container,
            content,
            layer,
            bounds: Cell::new(Bounds::default()),
        });
        let composition = window.enable_window_composition().map_err(host_error)?;
        let surface = composition
            .create_external_gpu_surface(attachment.clone())
            .map_err(host_error)?;

        Ok(Self {
            surface,
            attachment,
            window: window.window_handle(),
            cx: cx.to_async(),
        })
    }
    pub(super) fn metal_layer(&self) -> metal::MetalLayer {
        self.attachment.layer.clone()
    }
    pub(super) fn hide(&self) {
        self.attachment.container.setHidden(true);
    }
    pub(super) fn place(
        &self,
        bounds: Bounds<gpui::Pixels>,
        clip: Bounds<gpui::Pixels>,
        state: SurfaceState,
    ) -> Result<(), SurfaceError> {
        let attachment = self.surface.platform_surface().map_err(host_error)?;
        attachment
            .set_bounds(clip.to_device_pixels(state.scale))
            .map_err(host_error)?;
        let offset = bounds.origin - clip.origin;
        self.attachment.content.setFrame(NSRect::new(
            NSPoint::new(
                f64::from(offset.x),
                f64::from(clip.size.height - offset.y - bounds.size.height),
            ),
            NSSize::new(f64::from(bounds.size.width), f64::from(bounds.size.height)),
        ));
        // The producer configures drawableSize on its own thread; host supplies scale only.
        self.attachment
            .layer
            .set_contents_scale(f64::from(state.scale));
        attachment.set_visible(state.visible).map_err(host_error)?;
        // set_visible can skip repeated true values; removal may have hidden the view directly.
        self.attachment.container.setHidden(!state.visible);
        Ok(())
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.hide();
        self.attachment.container.removeFromSuperview();
        let surface = self.surface.clone();
        let window = self.window;
        self.cx
            .spawn(async move |cx| {
                let _ = window.update(cx, |_, window, _| {
                    window
                        .enable_window_composition()?
                        .remove_surface(surface.id())
                });
            })
            .detach();
    }
}

impl PlatformSurfaceAttachment for Attachment {
    fn set_bounds(&self, bounds: Bounds<DevicePixels>) -> anyhow::Result<()> {
        self.bounds.set(bounds);
        Ok(())
    }
    fn bounds(&self) -> Bounds<DevicePixels> {
        self.bounds.get()
    }
    fn set_parent_origin(&self, _: Point<DevicePixels>) -> anyhow::Result<()> {
        Ok(())
    }
    fn set_visible(&self, visible: bool) -> anyhow::Result<()> {
        self.container.setHidden(!visible);
        Ok(())
    }
    fn platform_handle(&self) -> anyhow::Result<Box<dyn Any>> {
        Ok(Box::new(Retained::as_ptr(&self.container) as usize))
    }
}

fn host_error(error: anyhow::Error) -> SurfaceError {
    SurfaceError(format!("{error:#}"))
}
