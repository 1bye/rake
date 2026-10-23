//! Explicit development-only checkpoint storage. Failed restoration never deletes it.

use super::{MAX_CHECKPOINT_BYTES, WorkspaceCheckpoint};
use crate::control::ControlConfig;

#[cfg(target_family = "wasm")]
const STORAGE_KEY: &str = "rake-dev-workspace";

pub(crate) fn load(config: ControlConfig) -> Option<Result<WorkspaceCheckpoint, String>> {
    if !matches!(config, ControlConfig::FromEnvironment) {
        return None;
    }

    #[cfg(not(target_family = "wasm"))]
    {
        use std::io::Read;
        use std::os::unix::fs::MetadataExt;
        use std::os::unix::fs::OpenOptionsExt;

        if std::env::var("RAKE_DEV_CONTROL").as_deref() != Ok("1") {
            return None;
        }

        let path = std::env::var_os("RAKE_RELOAD_CHECKPOINT")?;

        Some((|| {
            let mut options = std::fs::OpenOptions::new();

            options
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
            let file = options.open(path).map_err(|error| error.to_string())?;
            let metadata = file.metadata().map_err(|error| error.to_string())?;
            // SAFETY: geteuid has no preconditions and returns the current process identity.
            let uid = unsafe { libc::geteuid() };

            if !metadata.is_file() || metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
                return Err("reload checkpoint must be an owned private regular file".into());
            }

            let mut bytes = Vec::new();

            file.take(MAX_CHECKPOINT_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| error.to_string())?;

            decode(&bytes)
        })())
    }

    #[cfg(target_family = "wasm")]
    {
        match crate::control::WebBridgeConfig::from_document() {
            Ok(Some(_)) => {}
            _ => return None,
        }

        let storage = web_sys::window()?.session_storage().ok()??;
        let bytes = storage.get_item(STORAGE_KEY).ok()??;

        Some(decode(bytes.as_bytes()))
    }
}

fn decode(bytes: &[u8]) -> Result<WorkspaceCheckpoint, String> {
    if bytes.len() > MAX_CHECKPOINT_BYTES {
        return Err("reload checkpoint exceeds 512 KiB".into());
    }

    serde_json::from_slice(bytes).map_err(|error| error.to_string())
}

pub(crate) fn clear() {
    #[cfg(not(target_family = "wasm"))]
    if std::env::var("RAKE_DEV_CONTROL").as_deref() == Ok("1")
        && let Some(path) = std::env::var_os("RAKE_RELOAD_CHECKPOINT")
    {
        let _ = std::fs::remove_file(path);
    }

    #[cfg(target_family = "wasm")]
    if let Some(storage) =
        web_sys::window().and_then(|window| window.session_storage().ok().flatten())
    {
        let _ = storage.remove_item(STORAGE_KEY);
    }
}

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
