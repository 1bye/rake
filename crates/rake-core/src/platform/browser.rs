//! Browser extensions. Availability, permission and user activation are independent.

use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use super::{Capabilities, Capability, Error, PermissionState};

#[wasm_bindgen(module = "/src/platform/browser.js")]
extern "C" {
    #[wasm_bindgen(js_name = capabilities)]
    fn detect() -> u32;
    #[wasm_bindgen(js_name = clipboard_permission)]
    fn permission(write: bool) -> js_sys::Promise;
    #[wasm_bindgen(catch, js_name = preference_get)]
    fn storage_get(namespace: &str, key: &str) -> Result<Option<String>, JsValue>;
    #[wasm_bindgen(catch, js_name = preference_set)]
    fn storage_set(namespace: &str, key: &str, value: Option<&str>) -> Result<(), JsValue>;
}

#[doc(hidden)]
pub fn error(operation: &'static str, value: JsValue) -> Error {
    let string = |key| {
        js_sys::Reflect::get(&value, &JsValue::from_str(key))
            .ok()
            .and_then(|value| value.as_string())
    };
    match string("name").as_deref() {
        Some("NotSupportedError") => Error::Unsupported(operation),
        Some("NotAllowedError" | "SecurityError") => Error::PermissionDenied(operation),
        Some("QuotaExceededError") => Error::QuotaExceeded,
        _ => Error::Unavailable {
            operation,
            message: string("message").unwrap_or_else(|| format!("{value:?}")),
        },
    }
}

pub(super) fn capabilities() -> Capabilities {
    let bits = detect();
    let capability = |bit, gesture| Capability {
        supported: bits & bit != 0,
        permission: PermissionState::Unknown,
        requires_user_gesture: gesture,
    };
    Capabilities {
        notifications: super::notifications::capability(),
        global_shortcuts: super::GlobalShortcuts.capability(),
        file_import: capability(1, true),
        file_export: capability(2, true),
        clipboard_read: capability(4, true),
        clipboard_write: capability(8, true),
        preferences: capability(16, false),
        open_url: capability(32, true),
    }
}

/// Queries clipboard permission where supported. `Unknown` is expected on hosts
/// without this query; it does not imply that reading/writing is unavailable.
pub async fn clipboard_permission(write: bool) -> PermissionState {
    match JsFuture::from(permission(write))
        .await
        .ok()
        .and_then(|value| value.as_string())
        .as_deref()
    {
        Some("granted") => PermissionState::Granted,
        Some("prompt") => PermissionState::Prompt,
        Some("denied") => PermissionState::Denied,
        _ => PermissionState::Unknown,
    }
}

/// Reads text from a trusted DOM paste event without invoking clipboard permissions.
/// Do not suppress the browser's paste shortcut before this event can arrive.
pub fn text_from_paste_event(event: &web_sys::ClipboardEvent) -> Option<String> {
    event.clipboard_data()?.get_data("text/plain").ok()
}

pub(super) fn preference_get(namespace: &str, key: &str) -> Result<Option<String>, Error> {
    storage_get(namespace, key).map_err(|value| error("preferences read", value))
}

pub(super) fn preference_set(namespace: &str, key: &str, value: Option<&str>) -> Result<(), Error> {
    storage_set(namespace, key, value).map_err(|value| error("preferences write", value))
}
