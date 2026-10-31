//! Browser extensions. Availability, permission and user activation are independent.

use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use super::{Capabilities, Capability, Error, PermissionState};
#[cfg(feature = "gpui")]
use super::Request;

#[wasm_bindgen(module = "/src/platform/browser.js")]
extern "C" {
    #[wasm_bindgen(js_name = capabilities)]
    fn detect() -> u32;
    #[cfg(feature = "gpui")]
    #[wasm_bindgen(js_name = pick_file)]
    fn select_file() -> js_sys::Promise;
    #[cfg(feature = "gpui")]
    #[wasm_bindgen(js_name = read_file)]
    fn file_bytes(file: &JsValue, limit: f64) -> js_sys::Promise;
    #[cfg(feature = "gpui")]
    #[wasm_bindgen(catch, js_name = export_file)]
    fn download_file(name: &str, bytes: &[u8]) -> Result<(), JsValue>;
    #[cfg(feature = "gpui")]
    #[wasm_bindgen(js_name = read_clipboard)]
    fn clipboard_read() -> js_sys::Promise;
    #[cfg(feature = "gpui")]
    #[wasm_bindgen(js_name = write_clipboard)]
    fn clipboard_write(text: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_name = clipboard_permission)]
    fn permission(write: bool) -> js_sys::Promise;
    #[wasm_bindgen(catch, js_name = preference_get)]
    fn storage_get(namespace: &str, key: &str) -> Result<Option<String>, JsValue>;
    #[wasm_bindgen(catch, js_name = preference_set)]
    fn storage_set(namespace: &str, key: &str, value: Option<&str>) -> Result<(), JsValue>;
    #[cfg(feature = "gpui")]
    #[wasm_bindgen(catch, js_name = open_url)]
    fn launch_url(url: &str) -> Result<(), JsValue>;
}

pub(super) fn error(operation: &'static str, value: JsValue) -> Error {
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

#[cfg(feature = "gpui")]
pub(super) fn pick_file() -> Request<Option<super::FileHandle>> {
    let request = select_file(); // Initiate before yielding the event handler.
    Box::pin(async move {
        let file = JsFuture::from(request)
            .await
            .map_err(|value| error("file import", value))?;
        if file.is_null() || file.is_undefined() {
            return Ok(None);
        }
        let name = js_sys::Reflect::get(&file, &JsValue::from_str("name"))
            .ok()
            .and_then(|value| value.as_string())
            .ok_or_else(|| Error::Unavailable {
                operation: "file import",
                message: "host returned a file without a name".into(),
            })?;
        Ok(Some(super::FileHandle { name, file }))
    })
}

#[cfg(feature = "gpui")]
pub(super) async fn read_file(file: &JsValue, max_bytes: usize) -> Result<Vec<u8>, Error> {
    let bytes = JsFuture::from(file_bytes(file, max_bytes as f64))
        .await
        .map_err(|value| {
            if js_sys::Reflect::get(&value, &JsValue::from_str("name"))
                .ok()
                .and_then(|value| value.as_string())
                .as_deref()
                == Some("RakeSizeLimit")
            {
                Error::SizeLimit(max_bytes)
            } else {
                error("file read", value)
            }
        })?;
    Ok(js_sys::Uint8Array::new(&bytes).to_vec())
}

#[cfg(feature = "gpui")]
pub(super) fn export_file(name: &str, bytes: &[u8]) -> Result<(), Error> {
    download_file(name, bytes).map_err(|value| error("file export", value))
}

#[cfg(feature = "gpui")]
pub(super) fn read_clipboard() -> Request<Option<String>> {
    let request = clipboard_read();
    Box::pin(async move {
        let value = JsFuture::from(request)
            .await
            .map_err(|value| error("clipboard read", value))?;
        Ok(value.as_string())
    })
}

#[cfg(feature = "gpui")]
pub(super) fn write_clipboard(text: &str) -> Request<()> {
    let request = clipboard_write(text);
    Box::pin(async move {
        JsFuture::from(request)
            .await
            .map(|_| ())
            .map_err(|value| error("clipboard write", value))
    })
}

#[cfg(feature = "gpui")]
pub(super) fn open_url(url: &str) -> Result<(), Error> {
    launch_url(url).map_err(|value| error("open URL", value))
}
