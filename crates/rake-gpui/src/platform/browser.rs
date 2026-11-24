//! GPUI browser file, clipboard and activation bindings.

pub use rake_core::platform::browser::{clipboard_permission, text_from_paste_event};

use super::{Error, Request};
use rake_core::platform::browser::error;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen(module = "/src/platform/browser.js")]
extern "C" {
    #[wasm_bindgen(js_name = on_canvas_file_drop)]
    fn listen_canvas_drop(callback: &js_sys::Function) -> js_sys::Function;
    #[wasm_bindgen(js_name = pick_file)]
    fn select_file() -> js_sys::Promise;
    #[wasm_bindgen(js_name = read_file)]
    fn file_bytes(file: &JsValue, limit: f64) -> js_sys::Promise;
    #[wasm_bindgen(catch, js_name = export_file)]
    fn download_file(name: &str, bytes: &[u8]) -> Result<(), JsValue>;
    #[wasm_bindgen(js_name = read_clipboard)]
    fn clipboard_read() -> js_sys::Promise;
    #[wasm_bindgen(js_name = write_clipboard)]
    fn clipboard_write(text: &str) -> js_sys::Promise;
    #[wasm_bindgen(catch, js_name = open_url)]
    fn launch_url(url: &str) -> Result<(), JsValue>;
}

pub(super) fn pick_file() -> Request<Option<super::FileHandle>> {
    // Observe rejections even if work admission fails and the request is never polled.
    let request = JsFuture::from(select_file());
    Box::pin(async move {
        let file = request.await.map_err(|value| error("file import", value))?;
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

pub(super) fn export_file(name: &str, bytes: &[u8]) -> Result<(), Error> {
    download_file(name, bytes).map_err(|value| error("file export", value))
}

pub(super) fn read_clipboard() -> Request<Option<String>> {
    let request = JsFuture::from(clipboard_read());
    Box::pin(async move {
        let value = request
            .await
            .map_err(|value| error("clipboard read", value))?;
        Ok(value.as_string())
    })
}

pub(super) fn write_clipboard(text: &str) -> Request<()> {
    let request = JsFuture::from(clipboard_write(text));
    Box::pin(async move {
        request
            .await
            .map(|_| ())
            .map_err(|value| error("clipboard write", value))
    })
}

pub(super) fn open_url(url: &str) -> Result<(), Error> {
    launch_url(url).map_err(|value| error("open URL", value))
}

/// Owns canvas drop listeners. Retain it while the target view is mounted.
pub struct CanvasFileDrop {
    dispose: js_sys::Function,
    _callback: Closure<dyn FnMut(JsValue)>,
}

impl Drop for CanvasFileDrop {
    fn drop(&mut self) {
        let _ = self.dispose.call0(&JsValue::NULL);
    }
}

/// Registers a canvas-wide single-file target without inventing filesystem paths.
/// Empty and multi-file drops report invalid input. Reading remains caller bounded.
pub fn on_canvas_file_drop(
    mut callback: impl FnMut(Result<super::FileHandle, Error>) + 'static,
) -> CanvasFileDrop {
    let closure = Closure::wrap(Box::new(move |file: JsValue| {
        let name = js_sys::Reflect::get(&file, &JsValue::from_str("name"))
            .ok()
            .and_then(|name| name.as_string());
        callback(match name {
            Some(name) => Ok(super::FileHandle { name, file }),
            None => Err(Error::InvalidInput("drop exactly one file".into())),
        });
    }) as Box<dyn FnMut(JsValue)>);
    let dispose = listen_canvas_drop(closure.as_ref().unchecked_ref());

    CanvasFileDrop {
        dispose,
        _callback: closure,
    }
}
