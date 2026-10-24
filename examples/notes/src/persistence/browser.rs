//! Browser writes use Web Locks to serialize compare-and-write across documents.

use super::{Envelope, StorageError};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = r#"
function storageError(message) { throw new Error(message); }
function snapshots(key) { return ['library', 'recovery', 'previous'].map(s => localStorage.getItem(key + '.' + s)); }
async function locked(key, mode, work) {
  if (!navigator.locks) storageError('Web Locks are unavailable. Fieldnotes needs a secure browser context to protect your notes.');
  return navigator.locks.request(key, {mode}, work);
}
export async function fieldnotes_read(key) {
  return locked(key, 'shared', () => JSON.stringify(snapshots(key)));
}
export async function fieldnotes_write(key, raw, observed, checkpointOnly, keepPrevious) {
  return locked(key, 'exclusive', () => {
    // Rust validates formats, documents and revisions before this transaction.
    // Compare every raw snapshot under the lock so that validation cannot race a writer.
    const current = snapshots(key);
    if (JSON.stringify(current) !== observed) storageError('CONFLICT');
    // Each setItem is atomic; a quota failure leaves an older complete snapshot intact.
    localStorage.setItem(key + '.recovery', raw);
    if (!checkpointOnly) {
      if (keepPrevious && current[0] !== null) localStorage.setItem(key + '.previous', current[0]);
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
        observed: &str,
        checkpoint_only: bool,
        keep_previous: bool,
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
    envelope: &Envelope,
    raw: &str,
    expected: u64,
    checkpoint_only: bool,
) -> Result<(), StorageError> {
    let snapshots = read(key).await?;
    let current = super::current_snapshot(
        snapshots.0.as_deref(),
        snapshots.1.as_deref(),
        snapshots.2.as_deref(),
    )?;
    super::check_write(current.as_ref(), envelope, expected)?;
    let keep_previous = snapshots
        .0
        .as_deref()
        .is_some_and(|raw| super::decode(raw).is_ok());
    let observed = rake::serde_json::to_string(&snapshots)
        .map_err(|error| StorageError::Invalid(error.to_string()))?;
    wasm_bindgen_futures::JsFuture::from(fieldnotes_write(
        key,
        raw,
        &observed,
        checkpoint_only,
        keep_previous,
    ))
    .await
    .map_err(error)?;
    Ok(())
}
