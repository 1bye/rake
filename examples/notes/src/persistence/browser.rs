//! Browser writes use Web Locks to serialize compare-and-write across documents.

use super::StorageError;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = r#"
function storageError(message) { throw new Error(message); }
function snapshot(raw) {
  let value; try { value = JSON.parse(raw); } catch { return null; }
  if (!value || typeof value !== 'object') return null;
  if (Number.isSafeInteger(value.format) && value.format > 1) storageError('Unsupported library format ' + value.format + '. Your notes were preserved.');
  const notes = value.format === 0 ? value.notes : value.library?.notes;
  return Number.isSafeInteger(value.revision) && value.revision > 0 && Array.isArray(notes) ? value : null;
}
async function locked(key, mode, work) {
  if (!navigator.locks) storageError('Web Locks are unavailable. Fieldnotes needs a secure browser context to protect your notes.');
  return navigator.locks.request(key, {mode}, work);
}
export async function fieldnotes_read(key) {
  return locked(key, 'shared', () => JSON.stringify(['library', 'recovery', 'previous'].map(s => localStorage.getItem(key + '.' + s))));
}
export async function fieldnotes_write(key, raw, expected, checkpointOnly) {
  return locked(key, 'exclusive', () => {
    const incoming = JSON.parse(raw);
    let current = null;
    for (const suffix of ['library', 'recovery', 'previous']) {
      if (suffix === 'previous' && current) break;
      const stored = localStorage.getItem(key + '.' + suffix);
      if (!stored) continue;
      const value = snapshot(stored);
      if (!value) continue;
      if (!current || value.revision > current.revision) current = value;
    }
    const identical = current && current.revision === incoming.revision && JSON.stringify(current.library) === JSON.stringify(incoming.library);
    if (!identical && (current ? current.revision > incoming.revision || (current.revision !== expected && current.writer !== incoming.writer) : expected !== 0)) storageError('CONFLICT');
    // Each setItem is atomic; a quota failure leaves an older complete snapshot intact.
    localStorage.setItem(key + '.recovery', raw);
    if (!checkpointOnly) {
      const old = localStorage.getItem(key + '.library');
      if (old && snapshot(old)) localStorage.setItem(key + '.previous', old);
      localStorage.setItem(key + '.library', raw);
    }
  });
}
"#)]
extern "C" {
    fn fieldnotes_read(key: &str) -> js_sys::Promise;
    fn fieldnotes_write(
        key: &str,
        raw: &str,
        expected: f64,
        checkpoint_only: bool,
    ) -> js_sys::Promise;
}

fn error(value: JsValue) -> StorageError {
    let message = js_sys::Reflect::get(&value, &JsValue::from_str("message"))
        .ok()
        .and_then(|value| value.as_string())
        .unwrap_or_else(|| format!("{value:?}"));
    if message == "CONFLICT" {
        StorageError::Conflict
    } else {
        StorageError::Io(message)
    }
}

pub(super) async fn read(
    key: &str,
) -> Result<(Option<String>, Option<String>, Option<String>), StorageError> {
    let raw = wasm_bindgen_futures::JsFuture::from(fieldnotes_read(key))
        .await
        .map_err(error)?
        .as_string()
        .ok_or_else(|| StorageError::Invalid("Browser storage returned invalid data.".into()))?;
    rake::serde_json::from_str(&raw).map_err(|error| StorageError::Invalid(error.to_string()))
}

pub(super) async fn write(
    key: &str,
    raw: &str,
    expected: u64,
    checkpoint_only: bool,
) -> Result<(), StorageError> {
    wasm_bindgen_futures::JsFuture::from(fieldnotes_write(
        key,
        raw,
        expected as f64,
        checkpoint_only,
    ))
    .await
    .map_err(error)?;
    Ok(())
}
