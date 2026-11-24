//! Browser host requests with retained callback ownership and best-effort focus.

#[cfg(feature = "gpui")]
use super::{Error, Notification, NotificationOutcome, Platform};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(module = "/src/platform/notifications/browser.js")]
extern "C" {
    #[wasm_bindgen(catch, js_name = notification_permission)]
    fn query() -> Result<String, JsValue>;
    #[wasm_bindgen(js_name = create_host)]
    fn create(activate: &js_sys::Function) -> JsValue;
    #[wasm_bindgen(catch, js_name = show_notification)]
    fn post(host: &JsValue, tag: &str, title: &str, body: &str) -> Result<bool, JsValue>;
    #[wasm_bindgen(js_name = dismiss_notification)]
    fn dismiss(host: &JsValue, tag: &str);
    #[wasm_bindgen(js_name = dispose_host)]
    fn dispose(host: &JsValue);
}

#[cfg(feature = "gpui")]
pub(super) struct Host {
    handle: JsValue,
    _activate: Closure<dyn FnMut(String)>,
}

#[cfg(feature = "gpui")]
impl Drop for Host {
    fn drop(&mut self) {
        dispose(&self.handle);
    }
}

#[cfg(feature = "gpui")]
pub(super) fn install(platform: &Platform) -> Host {
    let activations = platform.core.open_url_receiver();
    let notifications = std::rc::Rc::downgrade(&platform.host().notifications);
    let activate = Closure::new(move |tag: String| {
        if let Some(notifications) = notifications.upgrade() {
            let payload = notifications.pending.borrow_mut().remove(&tag);

            if let Some(payload) = payload {
                activations(payload.as_str());
            }
        }
    });
    let handle = create(activate.as_ref().unchecked_ref());

    Host {
        handle,
        _activate: activate,
    }
}

#[cfg(feature = "gpui")]
pub(super) fn show(
    platform: &Platform,
    notification: Notification,
    payload: url::Url,
) -> Result<NotificationOutcome, Error> {
    let permission =
        query().map_err(|error| rake_core::platform::browser::error("notifications", error))?;

    if permission != "granted" {
        return Ok(NotificationOutcome::Denied);
    }

    // Reserve before posting, restoring the previous payload if the host fails.
    let previous = platform
        .host()
        .notifications
        .pending
        .borrow()
        .get(&notification.tag)
        .cloned();
    platform.remember(&notification.tag, payload)?;
    let host = platform.host().notifications.browser.borrow();
    let host = host
        .as_ref()
        .ok_or(Error::Unsupported("notification host"))?;
    let result = post(
        &host.handle,
        &notification.tag,
        &notification.title,
        &notification.body,
    );

    match result {
        Ok(true) => Ok(NotificationOutcome::Submitted),
        result => {
            let mut pending = platform.host().notifications.pending.borrow_mut();
            pending.remove(&notification.tag);

            if let Some(previous) = previous {
                pending.insert(notification.tag, previous);
            }

            match result {
                Ok(_) => Ok(NotificationOutcome::Denied),
                Err(error) => Err(rake_core::platform::browser::error("notifications", error)),
            }
        }
    }
}

#[cfg(feature = "gpui")]
pub(super) fn dismiss_tag(platform: &Platform, tag: &str) {
    if let Some(host) = platform.host().notifications.browser.borrow().as_ref() {
        dismiss(&host.handle, tag);
    }
}
