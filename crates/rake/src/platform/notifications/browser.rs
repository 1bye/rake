//! Browser host requests with retained callback ownership and best-effort focus.

use super::{Capability, PermissionState, Request};
#[cfg(feature = "gpui")]
use super::{Error, Notification, NotificationOutcome, Platform};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen(module = "/src/platform/notifications/browser.js")]
extern "C" {
    #[wasm_bindgen(catch, js_name = notification_permission)]
    fn query() -> Result<String, JsValue>;
    #[wasm_bindgen(js_name = request_notification_permission)]
    fn request() -> js_sys::Promise;
    #[wasm_bindgen(js_name = create_host)]
    fn create(activate: &js_sys::Function) -> JsValue;
    #[wasm_bindgen(catch, js_name = show_notification)]
    fn post(host: &JsValue, tag: &str, title: &str, body: &str) -> Result<bool, JsValue>;
    #[wasm_bindgen(js_name = dismiss_notification)]
    fn dismiss(host: &JsValue, tag: &str);
    #[wasm_bindgen(js_name = dispose_host)]
    fn dispose(host: &JsValue);
}

fn state(value: &str) -> PermissionState {
    match value {
        "granted" => PermissionState::Granted,
        "denied" => PermissionState::Denied,
        "default" => PermissionState::Prompt,
        _ => PermissionState::Unknown,
    }
}

pub(super) fn capability() -> Capability {
    let permission = query();

    Capability {
        supported: permission.is_ok(),
        permission: permission
            .as_deref()
            .map(state)
            .unwrap_or(PermissionState::Unknown),
        requires_user_gesture: true,
    }
}

pub(super) fn permission(prompt: bool) -> Request<PermissionState> {
    if prompt {
        let request = JsFuture::from(request());

        Box::pin(async move {
            let value = request.await.map_err(|error| {
                super::super::browser::error("notification authorization", error)
            })?;

            Ok(state(value.as_string().as_deref().unwrap_or("unknown")))
        })
    } else {
        let result = query()
            .map(|value| state(&value))
            .map_err(|error| super::super::browser::error("notifications", error));

        Box::pin(async { result })
    }
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
    let activations = std::rc::Rc::downgrade(&platform.activations);
    let notifications = std::rc::Rc::downgrade(&platform.notifications);
    let activate = Closure::new(move |tag: String| {
        if let (Some(activations), Some(notifications)) =
            (activations.upgrade(), notifications.upgrade())
        {
            let payload = notifications.pending.borrow_mut().remove(&tag);

            if let Some(payload) = payload {
                let platform = Platform {
                    activations,
                    notifications,
                };
                let _ = platform.receive_open_url(payload.as_str());
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
        query().map_err(|error| super::super::browser::error("notifications", error))?;

    if permission != "granted" {
        return Ok(NotificationOutcome::Denied);
    }

    // Reserve before posting, restoring the previous payload if the host fails.
    let previous = platform
        .notifications
        .pending
        .borrow()
        .get(&notification.tag)
        .cloned();
    platform.remember(&notification.tag, payload)?;
    let host = platform.notifications.browser.borrow();
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
            let mut pending = platform.notifications.pending.borrow_mut();
            pending.remove(&notification.tag);

            if let Some(previous) = previous {
                pending.insert(notification.tag, previous);
            }

            match result {
                Ok(_) => Ok(NotificationOutcome::Denied),
                Err(error) => Err(super::super::browser::error("notifications", error)),
            }
        }
    }
}

#[cfg(feature = "gpui")]
pub(super) fn dismiss_tag(platform: &Platform, tag: &str) {
    if let Some(host) = platform.notifications.browser.borrow().as_ref() {
        dismiss(&host.handle, tag);
    }
}
