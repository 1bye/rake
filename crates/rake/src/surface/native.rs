//! Linux and Windows attachments created and managed by GPUI Fast.

use gpui::{AnyWindowHandle, App, AsyncApp, Bounds, Pixels, Window, WindowCompositionSurface};
#[cfg(target_os = "linux")]
use raw_window_handle::{HasDisplayHandle as _, RawDisplayHandle, RawWindowHandle};
#[cfg(target_os = "windows")]
use std::rc::Rc;
#[cfg(target_os = "windows")]
use windows::core::{IUnknown, Interface as _};

use super::{SurfaceError, SurfaceState, SurfaceTarget};

pub(super) struct Host {
    surface: WindowCompositionSurface,
    window: AnyWindowHandle,
    cx: AsyncApp,
    #[cfg(target_os = "linux")]
    display: RawDisplayHandle,
}

impl Host {
    pub(super) fn new(window: &mut Window, cx: &mut App) -> Result<Self, SurfaceError> {
        #[cfg(target_os = "linux")]
        let display = window
            .display_handle()
            .map_err(|error| SurfaceError(format!("obtaining the GPUI display: {error}")))?
            .as_raw();

        let composition = window.enable_window_composition().map_err(host_error)?;
        let surface = composition.create_native_surface().map_err(host_error)?;
        let host = Self {
            surface,
            window: window.window_handle(),
            cx: cx.to_async(),
            #[cfg(target_os = "linux")]
            display,
        };
        host.set_visible(false)?;
        Ok(host)
    }

    pub(super) fn target(&self) -> Result<SurfaceTarget, SurfaceError> {
        let handle = self
            .surface
            .platform_surface()
            .and_then(|attachment| attachment.platform_handle())
            .map_err(host_error)?;

        #[cfg(target_os = "linux")]
        {
            let window = *handle.downcast::<RawWindowHandle>().map_err(|_| {
                SurfaceError("GPUI did not supply a Linux child window handle".into())
            })?;
            Ok(SurfaceTarget::Linux {
                window,
                display: self.display,
            })
        }
        #[cfg(target_os = "windows")]
        {
            let visual = *handle.downcast::<IUnknown>().map_err(|_| {
                SurfaceError("GPUI did not supply a DirectComposition visual".into())
            })?;
            let visual = visual.cast().map_err(|error| {
                SurfaceError(format!("obtaining the DirectComposition visual: {error}"))
            })?;
            Ok(SurfaceTarget::DirectComposition(visual))
        }
    }

    #[cfg(target_os = "windows")]
    pub(super) fn commit_target(&self, window: &Window) -> Result<(), SurfaceError> {
        if self.window.window_id() != window.window_handle().window_id() {
            return Err(SurfaceError("GPU surface belongs to another window".into()));
        }
        // Enabling composition also synchronizes the existing visual tree.
        // Fast commits the DirectComposition device as part of that operation.
        window
            .enable_window_composition()
            .map(|_| ())
            .map_err(host_error)
    }

    #[cfg(target_os = "windows")]
    pub(super) fn on_compositor_recreated(
        &self,
        callback: impl Fn() + 'static,
    ) -> Result<(), SurfaceError> {
        self.surface
            .platform_surface()
            .and_then(|attachment| {
                attachment.set_compositor_recreated_callback(Rc::new(move |_| {
                    callback();
                    Ok(())
                }))
            })
            .map_err(host_error)
    }

    fn set_visible(&self, visible: bool) -> Result<(), SurfaceError> {
        self.surface
            .platform_surface()
            .and_then(|attachment| attachment.set_visible(visible))
            .map_err(host_error)
    }

    pub(super) fn hide(&self) {
        if let Err(error) = self.set_visible(false) {
            tracing::debug!(%error, "GPU surface could not be hidden");
        }
    }

    pub(super) fn place(
        &self,
        _: Bounds<Pixels>,
        clip: Bounds<Pixels>,
        state: SurfaceState,
    ) -> Result<(), SurfaceError> {
        self.surface
            .platform_surface()
            .and_then(|attachment| {
                attachment.set_bounds(clip.to_device_pixels(state.scale))?;
                attachment.set_visible(state.visible)
            })
            .map_err(host_error)
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.hide();
        let id = self.surface.id();
        let window = self.window;
        self.cx
            .spawn(async move |cx| {
                let _ = window.update(cx, |_, window, _| {
                    window.enable_window_composition()?.remove_surface(id)
                });
            })
            .detach();
    }
}

fn host_error(error: anyhow::Error) -> SurfaceError {
    SurfaceError(format!("{error:#}"))
}
