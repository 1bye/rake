//! Host visibility adapters. They report surface visibility, never keyboard focus.

use super::HostVisibility;
use gpui::{App, Window};
use std::rc::Rc;

pub(crate) struct HostObserver {
    _quit: gpui::Subscription,
    #[cfg(target_os = "macos")]
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

        // shortcut: GPUI cannot veto OS/Dock termination here; use a cancelable
        // application-quit hook when GPUI provides one.
        let quit = cx.on_app_quit(move |_| {
            shutdown();

            async {}
        });

        #[cfg(target_os = "macos")]
        {
            report(native_visibility(window));
            let mut context = window.to_async(cx);

            // GPUI currently exposes no occlusion observer. Poll the actual AppKit
            // surface on the UI loop rather than substituting active-window status.
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

        #[cfg(not(any(target_os = "macos", target_family = "wasm")))]
        {
            let _ = (window, cx);

            report(HostVisibility::Unknown);

            Self { _quit: quit }
        }
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
