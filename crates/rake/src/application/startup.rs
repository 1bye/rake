//! Keeps callback failures observable across the platform event-loop boundary.
use std::{cell::RefCell, rc::Rc};

use super::AppError;

#[derive(Clone, Default)]
pub(super) struct StartupErrors(Rc<RefCell<Option<AppError>>>);

impl StartupErrors {
    pub(super) fn record(&self, error: AppError) {
        let mut failure = self.0.borrow_mut();

        if failure.is_none() {
            #[cfg(target_family = "wasm")]
            show_browser_error(&error.to_string());
            *failure = Some(error);
        }
    }

    pub(super) fn failed(&self) -> bool {
        self.0.borrow().is_some()
    }

    pub(super) fn finish(&self) -> Result<(), AppError> {
        self.0.borrow_mut().take().map_or(Ok(()), Err)
    }
}

#[cfg(target_family = "wasm")]
type ReloadHandler = wasm_bindgen::closure::Closure<dyn FnMut(web_sys::Event)>;

#[cfg(target_family = "wasm")]
thread_local! {
    static RELOAD_HANDLER: RefCell<Option<(web_sys::Element, ReloadHandler)>> = const { RefCell::new(None) };
}

#[cfg(target_family = "wasm")]
pub(super) fn clear_browser_error() {
    use wasm_bindgen::JsCast as _;
    RELOAD_HANDLER.with(|slot| {
        if let Some((button, handler)) = slot.borrow_mut().take() {
            let _ = button
                .remove_event_listener_with_callback("click", handler.as_ref().unchecked_ref());
        }
    });

    if let Some(old) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("rake-startup-error"))
    {
        old.remove();
    }
}

// This fallback works even when GPUI cannot create a graphics surface. Error
// text is never interpreted as HTML; the reload handler lives with the panel.
#[cfg(target_family = "wasm")]
pub(super) fn show_browser_error(message: &str) {
    use wasm_bindgen::JsCast as _;

    let Some(window) = web_sys::window() else {
        return;
    };

    let Some(document) = window.document() else {
        return;
    };

    let Some(body) = document.body() else { return };

    clear_browser_error();

    let Ok(panel) = document.create_element("section") else {
        return;
    };

    panel.set_id("rake-startup-error");
    let _ = panel.set_attribute("role", "alert");
    let _ = panel.set_attribute("style", "position:fixed;inset:0;z-index:2147483647;background:#18232b;color:#f0f4e8;padding:24px;white-space:pre-wrap;overflow:auto");

    panel.set_text_content(Some(&format!(
        "Application could not start\n\n{message}\n\n"
    )));

    if let Ok(reload) = document.create_element("button") {
        let _ = reload.set_attribute("type", "button");

        reload.set_text_content(Some("Reload"));
        let handler = ReloadHandler::new(move |_| {
            let _ = window.location().reload();
        });

        if reload
            .add_event_listener_with_callback("click", handler.as_ref().unchecked_ref())
            .is_ok()
        {
            let _ = panel.append_child(&reload);

            RELOAD_HANDLER.with(|slot| *slot.borrow_mut() = Some((reload, handler)));
        }
    }

    let _ = body.append_child(&panel);
}
