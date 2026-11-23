//! GPUI host services and application permissions.

pub use files::{FileHandle, SaveOutcome};

pub use rake_core::platform::{
    Capabilities, Capability, Error, Notification, NotificationOutcome, PermissionState,
    Preferences, Request, Secret,
};

pub use shortcuts::{GlobalShortcut, GlobalShortcuts};

mod files;
pub mod icons;
mod notifications;
mod secrets;

pub use secrets::Secrets;

mod shortcuts;

#[cfg(target_family = "wasm")]
pub mod browser;

#[derive(Default)]
struct Host {
    shortcuts: GlobalShortcuts,
    notifications: std::rc::Rc<notifications::Notifications>,
}

/// Shared GPUI platform services and URL activation inbox.
#[derive(Clone, Debug, Default)]
pub struct Platform {
    core: rake_core::platform::Platform,
}

impl Platform {
    pub(crate) fn for_application(application_id: &str) -> Self {
        Self::from_core(rake_core::platform::Platform::for_application(
            application_id,
        ))
    }

    pub(crate) fn from_core(core: rake_core::platform::Platform) -> Self {
        Self { core }
    }

    pub(crate) fn into_core(self) -> rake_core::platform::Platform {
        self.core
    }

    fn host(&self) -> &Host {
        self.core.host_state(Host::default)
    }

    /// Detects current host availability without prompting for permission.
    pub fn capabilities(&self) -> Capabilities {
        #[cfg(target_family = "wasm")]
        {
            let mut capabilities = self.core.capabilities();

            capabilities.global_shortcuts = self.host().shortcuts.capability();

            capabilities
        }

        #[cfg(not(target_family = "wasm"))]
        {
            let mut capabilities = self.core.capabilities();

            let ui = Capability {
                supported: true,
                permission: PermissionState::NotRequired,
                requires_user_gesture: false,
            };

            capabilities.file_import = ui;
            capabilities.file_export = ui;
            capabilities.clipboard_read = ui;
            capabilities.clipboard_write = ui;
            capabilities.open_url = ui;
            capabilities.notifications = notifications::capability();
            capabilities.global_shortcuts = self.host().shortcuts.capability();

            capabilities
        }
    }

    /// Initiates a clipboard read. An empty clipboard yields `None` on native;
    /// an empty text value is `Some("")`. Browser callers should also accept paste
    /// events, since programmatic clipboard permission varies between browsers.
    #[cfg(feature = "gpui")]
    pub fn read_clipboard(&self, cx: &gpui::App) -> Request<Option<String>> {
        #[cfg(target_family = "wasm")]
        {
            let _ = cx;

            browser::read_clipboard()
        }

        #[cfg(not(target_family = "wasm"))]
        {
            let value = cx.read_from_clipboard().and_then(|item| item.text());

            Box::pin(async move { Ok(value) })
        }
    }

    /// Initiates a clipboard write. Failure is observable on browser hosts;
    /// native GPUI delegates to the system without a completion acknowledgement.
    #[cfg(feature = "gpui")]
    pub fn write_clipboard(&self, text: &str, cx: &gpui::App) -> Request<()> {
        #[cfg(target_family = "wasm")]
        {
            let _ = cx;

            browser::write_clipboard(text)
        }

        #[cfg(not(target_family = "wasm"))]
        {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.to_owned()));

            Box::pin(async { Ok(()) })
        }
    }

    /// Opens an HTTP, HTTPS or mailto URL. Browser popup denial is an error;
    /// native GPUI acknowledges delegation, not successful external navigation.
    ///
    /// # Errors
    /// Rejects malformed/other-scheme URLs and browser permission failures.
    #[cfg(feature = "gpui")]
    pub fn open_url(&self, value: &str, cx: &gpui::App) -> Result<(), Error> {
        let url = external_url(value)?;

        #[cfg(target_family = "wasm")]
        {
            let _ = cx;

            browser::open_url(url.as_str())
        }

        #[cfg(not(target_family = "wasm"))]
        {
            cx.open_url(url.as_str());

            Ok(())
        }
    }
}

fn external_url(value: &str) -> Result<url::Url, Error> {
    let url = url::Url::parse(value).map_err(|error| Error::InvalidInput(error.to_string()))?;

    if !matches!(url.scheme(), "https" | "http" | "mailto") {
        return Err(Error::InvalidInput(
            "external URL must use http, https or mailto".into(),
        ));
    }

    if !url.username().is_empty() || url.password().is_some() {
        return Err(Error::InvalidInput(
            "external URLs cannot contain credentials".into(),
        ));
    }

    Ok(url)
}

impl Platform {
    /// Opens an explicitly namespaced preference store. Values use last-writer-wins
    /// replacement; this is not a document database or a multi-key transaction.
    ///
    /// # Errors
    /// Rejects empty/traversal names or an unavailable native application-data root.
    pub fn preferences(&self, namespace: &str) -> Result<Preferences, Error> {
        self.core.preferences(namespace)
    }

    /// Queues a validated application URL received from a host or custom launcher.
    /// The oldest activation is discarded beyond 32 pending URLs. This does not
    /// navigate automatically; the application owns scheme/host/argument validation.
    ///
    /// # Errors
    /// Rejects malformed or oversized URLs before queuing them.
    pub fn receive_open_url(&self, value: &str) -> Result<(), Error> {
        self.core.receive_open_url(value)
    }

    /// Waits for the next activation. Clones share one inbox; competing consumers
    /// divide events. Keep one application-owned consumer and cancel it at shutdown.
    pub async fn next_open_url(&self) -> url::Url {
        self.core.next_open_url().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_urls_reject_executable_schemes_and_credentials() {
        for invalid in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,hi",
            "https://user:secret@example.com",
        ] {
            assert!(external_url(invalid).is_err(), "accepted {invalid}");
        }

        assert!(external_url("https://example.com/path?q=a").is_ok());
        assert!(external_url("mailto:hello@example.com").is_ok());
    }
}
