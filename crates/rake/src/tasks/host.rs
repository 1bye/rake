//! Host visibility adapters. They report surface visibility, never keyboard focus.

use super::HostVisibility;
use gpui::{App, Window};
use std::rc::Rc;

pub(crate) struct HostObserver {
    _quit: gpui::Subscription,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    _poll: gpui::Task<()>,
    #[cfg(target_family = "wasm")]
    _browser: Option<BrowserVisibility>,
}

impl HostObserver {
    pub(crate) fn attach(
        report: impl Fn(HostVisibility) + 'static,
        shutdown: impl Fn() + 'static,
        warn_before_unload: impl Fn() -> bool + 'static,
        window: &Window,
        cx: &App,
    ) -> Self {
        let report = Rc::new(report);

        #[cfg(not(target_family = "wasm"))]
        let _ = warn_before_unload;

        // Managed host bridges handle cancelable quit before this final teardown.
        let quit = cx.on_app_quit(move |_| {
            shutdown();

            async {}
        });

        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            report(native_visibility(window));
            let mut context = window.to_async(cx);

            // GPUI currently exposes no visibility observer. Query the borrowed
            // native surface on its UI loop instead of substituting keyboard focus.
            let poll = cx.foreground_executor().spawn(async move {
                loop {
                    if context
                        .update(|window, _| report(native_visibility(window)))
                        .is_err()
                    {
                        break;
                    }

                    super::sleep(std::time::Duration::from_millis(250)).await;
                }
            });

            Self {
                _quit: quit,
                _poll: poll,
            }
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = (window, cx);

            Self {
                _quit: quit,
                _browser: BrowserVisibility::attach(report, warn_before_unload),
            }
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows", target_family = "wasm")))]
        {
            let _ = (window, cx);

            report(HostVisibility::Unknown);

            Self { _quit: quit }
        }
    }
}

#[cfg(target_os = "windows")]
fn native_visibility(window: &Window) -> HostVisibility {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
    use windows_sys::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindow, IsWindowVisible};
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return HostVisibility::Unknown;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return HostVisibility::Unknown;
    };
    let hwnd = handle.hwnd.get() as *mut std::ffi::c_void;
    let mut cloaked = 0_u32;
    // SAFETY: The borrowed GPUI window supplies a live HWND on the UI thread.
    // The DWM output points to an initialized u32 with exactly the requested size.
    unsafe {
        if IsWindow(hwnd) == 0 {
            return HostVisibility::Unknown;
        }
        if IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
            return HostVisibility::Hidden;
        }
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED as u32,
            (&mut cloaked as *mut u32).cast(),
            std::mem::size_of::<u32>() as u32,
        ) < 0
        {
            return HostVisibility::Unknown;
        }
    }
    if cloaked != 0 {
        HostVisibility::Hidden
    } else {
        HostVisibility::Visible
    }
}

#[cfg(target_os = "macos")]
fn native_visibility(window: &Window) -> HostVisibility {
    use objc2_app_kit::{NSView, NSWindowOcclusionState};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return HostVisibility::Unknown;
    };

    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return HostVisibility::Unknown;
    };

    // SAFETY: GPUI's borrowed AppKit window handle contains a live NSView. The
    // Window borrow keeps it alive, and this function runs inside a UI-loop update.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };

    let Some(window) = view.window() else {
        return HostVisibility::Hidden;
    };

    if window.isVisible()
        && !window.isMiniaturized()
        && window
            .occlusionState()
            .contains(NSWindowOcclusionState::Visible)
    {
        HostVisibility::Visible
    } else {
        HostVisibility::Hidden
    }
}

#[cfg(target_family = "wasm")]
struct BrowserVisibility {
    document: web_sys::Document,
    window: web_sys::Window,
    listener: wasm_bindgen::closure::Closure<dyn FnMut(web_sys::Event)>,
    before_unload: wasm_bindgen::closure::Closure<dyn FnMut(web_sys::BeforeUnloadEvent)>,
}

#[cfg(target_family = "wasm")]
impl BrowserVisibility {
    fn attach(
        report: Rc<dyn Fn(HostVisibility)>,
        warn: impl Fn() -> bool + 'static,
    ) -> Option<Self> {
        use wasm_bindgen::JsCast;
        let window = web_sys::window()?;
        let document = window.document()?;

        report(if document.hidden() {
            HostVisibility::Hidden
        } else {
            HostVisibility::Visible
        });
        let observed = document.clone();

        let listener = wasm_bindgen::closure::Closure::new(move |event: web_sys::Event| {
            report(if event.type_() == "pagehide" || observed.hidden() {
                HostVisibility::Hidden
            } else {
                HostVisibility::Visible
            });
        });

        let before_unload =
            wasm_bindgen::closure::Closure::new(move |event: web_sys::BeforeUnloadEvent| {
                if warn() {
                    event.prevent_default();
                    event.set_return_value("");
                }
            });

        let observer = Self {
            document,
            window,
            listener,
            before_unload,
        };

        let callback = observer.listener.as_ref().unchecked_ref();

        if observer
            .window
            .add_event_listener_with_callback(
                "beforeunload",
                observer.before_unload.as_ref().unchecked_ref(),
            )
            .is_err()
            || observer
                .document
                .add_event_listener_with_callback("visibilitychange", callback)
                .is_err()
            || observer
                .window
                .add_event_listener_with_callback("pagehide", callback)
                .is_err()
            || observer
                .window
                .add_event_listener_with_callback("pageshow", callback)
                .is_err()
        {
            crate::error!("Could not install browser visibility listeners");

            return None;
        }

        Some(observer)
    }
}

#[cfg(target_family = "wasm")]
impl Drop for BrowserVisibility {
    fn drop(&mut self) {
        use wasm_bindgen::JsCast;
        let _ = self.window.remove_event_listener_with_callback(
            "beforeunload",
            self.before_unload.as_ref().unchecked_ref(),
        );

        let callback = self.listener.as_ref().unchecked_ref();

        let _ = self
            .document
            .remove_event_listener_with_callback("visibilitychange", callback);

        for event in ["pagehide", "pageshow"] {
            let _ = self
                .window
                .remove_event_listener_with_callback(event, callback);
        }
    }
}
