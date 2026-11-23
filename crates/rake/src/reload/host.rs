//! Explicit development-only checkpoint storage. Failed restoration never deletes it.

#[cfg(any(unix, windows, target_family = "wasm"))]
use super::MAX_CHECKPOINT_BYTES;
use super::WorkspaceCheckpoint;
#[cfg(feature = "gpui")]
use crate::control::ControlConfig;

#[cfg(target_family = "wasm")]
const STORAGE_KEY: &str = "rake-dev-workspace";

#[cfg(feature = "gpui")]
pub(crate) fn load(config: ControlConfig) -> Option<Result<WorkspaceCheckpoint, String>> {
    if !matches!(config, ControlConfig::FromEnvironment) {
        return None;
    }

    #[cfg(unix)]
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

    #[cfg(windows)]
    {
        use std::io::Read;

        if std::env::var("RAKE_DEV_CONTROL").as_deref() != Ok("1") {
            return None;
        }

        let path = std::env::var_os("RAKE_RELOAD_CHECKPOINT")?;

        Some((|| {
            let file = rake_client::private::open(std::path::Path::new(&path))
                .map_err(|error| error.to_string())?;

            let mut bytes = Vec::new();

            file.take(MAX_CHECKPOINT_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| error.to_string())?;

            decode(&bytes)
        })())
    }

    // Checkpoints are owned by the existing Unix development supervisor.
    #[cfg(all(not(any(unix, windows)), not(target_family = "wasm")))]
    {
        None
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

#[cfg(all(feature = "gpui", any(unix, windows, target_family = "wasm")))]
fn decode(bytes: &[u8]) -> Result<WorkspaceCheckpoint, String> {
    if bytes.len() > MAX_CHECKPOINT_BYTES {
        return Err("reload checkpoint exceeds 512 KiB".into());
    }

    serde_json::from_slice(bytes).map_err(|error| error.to_string())
}

#[cfg(feature = "gpui")]
pub(crate) fn clear() {
    #[cfg(any(unix, windows))]
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

#[cfg(feature = "gpui")]
pub(crate) async fn restore<R: crate::router::Route>(
    router: &crate::router::Router<R>,
    checkpoint: Option<Result<WorkspaceCheckpoint, String>>,
    application_id: &str,
) {
    let Some(checkpoint) = checkpoint else { return };

    match checkpoint {
        Ok(checkpoint) => match router.restore_workspace(&checkpoint, application_id).await {
            Ok(()) => clear(),
            Err(error) => router.reload_failed(error.to_string()),
        },
        Err(error) => router.reload_failed(error),
    }
}

#[cfg(feature = "gpui")]
pub(crate) use ui::ReloadUi;

#[cfg(feature = "gpui")]
mod ui {
    use std::cell::RefCell;
    use std::collections::BTreeSet;
    use std::rc::Rc;

    use crate::router::{Navigator, Route, WindowId};
    use crate::tasks::HostVisibility;

    #[derive(Default)]
    pub(crate) struct ReloadUi {
        pending: RefCell<Rc<RefCell<BTreeSet<WindowId>>>>,
    }

    impl ReloadUi {
        pub(crate) fn begin(&self, windows: Vec<WindowId>) {
            // Frames queued for a previous request can only acknowledge that request.
            self.pending
                .replace(Rc::new(RefCell::new(windows.into_iter().collect())));
        }

        pub(crate) fn ready(
            &self,
            visibility: &dyn Fn(WindowId) -> Option<HostVisibility>,
        ) -> bool {
            self.pending
                .borrow()
                .borrow()
                .iter()
                .all(|&id| matches!(visibility(id), None | Some(HostVisibility::Hidden)))
        }

        pub(crate) fn observe_frame<R: Route>(
            &self,
            id: WindowId,
            window: &mut gpui::Window,
            navigator: Navigator<R>,
        ) {
            let pending = self.pending.borrow().clone();

            window.on_next_frame(move |_, _| {
                if pending.borrow_mut().remove(&id)
                    && let Some(router) = navigator.upgrade_router()
                {
                    router.reload_readiness_changed();
                }
            });
        }
    }
}

/// Transient live status shares the existing managed/routed reload presentation.
#[cfg(feature = "gpui")]
pub(crate) fn status_notice(cx: &gpui::App) -> Option<gpui::AnyElement> {
    #[cfg(all(feature = "live", not(target_family = "wasm")))]
    {
        super::live::status_notice(cx)
    }

    #[cfg(any(not(feature = "live"), target_family = "wasm"))]
    {
        let _ = cx;

        None
    }
}
