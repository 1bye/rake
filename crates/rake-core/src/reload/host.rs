//! Browser development checkpoint transport.
use super::{MAX_CHECKPOINT_BYTES, WorkspaceCheckpoint};

#[doc(hidden)]
pub const STORAGE_KEY: &str = "rake-dev-workspace";

#[cfg(target_family = "wasm")]
pub(crate) fn store(checkpoint: &WorkspaceCheckpoint) -> Result<(), String> {
    let bytes = serde_json::to_string(checkpoint).map_err(|error| error.to_string())?;

    if bytes.len() > MAX_CHECKPOINT_BYTES {
        return Err("reload checkpoint exceeds 512 KiB".into());
    }

    web_sys::window()
        .ok_or("browser unavailable")?
        .session_storage()
        .map_err(|error| format!("{error:?}"))?
        .ok_or("session storage unavailable")?
        .set_item(STORAGE_KEY, &bytes)
        .map_err(|error| format!("{error:?}"))
}
