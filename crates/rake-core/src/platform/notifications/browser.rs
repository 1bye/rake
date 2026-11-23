//! Browser host requests with retained callback ownership and best-effort focus.

use super::{Capability, PermissionState, Request};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen(module = "/src/platform/notifications/browser.js")]
extern "C" {
    #[wasm_bindgen(catch, js_name = notification_permission)]
    fn query() -> Result<String, JsValue>;
    #[wasm_bindgen(js_name = request_notification_permission)]
    fn request() -> js_sys::Promise;
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
