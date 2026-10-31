//! Portable host services with explicit support, cancellation and permission boundaries.
//!
//! Obtain the shared handle from [`crate::Services::platform`]. Picker and clipboard
//! methods initiate requests immediately, before their returned future is polled;
//! call them in a user event, then await in owned work. Dropping a request stops
//! observation, but cannot undo an OS dialog, clipboard write or browser download.

mod preferences;

pub use preferences::Preferences;

#[cfg(feature = "gpui")]
mod files;

#[cfg(feature = "gpui")]
pub use files::{FileHandle, SaveOutcome};

#[cfg(target_family = "wasm")]
pub mod browser;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

/// An initiated host request; browser work may require the initiating user event.
pub type Request<T> = Pin<Box<dyn Future<Output = Result<T, Error>>>>;

/// A host operation failed without silently substituting another behavior.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No implementation exists in the current host.
    #[error("{0} is unsupported by this host")]
    Unsupported(&'static str),
    /// Host security policy or the user denied the operation.
    #[error("permission denied for {0}")]
    PermissionDenied(&'static str),
    /// The host exhausted its storage allocation.
    #[error("storage quota exceeded")]
    QuotaExceeded,
    /// A value or name is invalid before any host operation starts.
    #[error("invalid platform request: {0}")]
    InvalidInput(String),
    /// A file exceeds the caller's byte limit.
    #[error("file exceeds the {0}-byte limit")]
    SizeLimit(usize),
    /// A native filesystem operation failed; the original cause is preserved.
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// A supported service is temporarily unavailable or failed.
    #[error("{operation} failed: {message}")]
    Unavailable {
        /// The failing operation, without user content.
        operation: &'static str,
        /// The host's diagnostic message.
        message: String,
    },
}

/// Permission knowledge is independent of API availability and user activation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionState {
    /// The service has no separate permission query.
    NotRequired,
    /// The host cannot reliably report permission before the request.
    Unknown,
    /// The host reports permission granted; an operation can still fail.
    Granted,
    /// A request may prompt the user.
    Prompt,
    /// The host reports permission denied.
    Denied,
}

/// A snapshot of one operation's availability, not a promise of success.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capability {
    /// Whether this host implements the operation.
    pub supported: bool,
    /// What is currently known about permission.
    pub permission: PermissionState,
    /// Whether callers should initiate directly in a click/key/paste event.
    pub requires_user_gesture: bool,
}

/// Runtime-detected host services. Browser snapshots can change with permissions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// Read a user-selected file.
    pub file_import: Capability,
    /// Select a native save path or request a browser download.
    pub file_export: Capability,
    /// Read clipboard text; paste events remain a separate browser input path.
    pub clipboard_read: Capability,
    /// Write clipboard text.
    pub clipboard_write: Capability,
    /// Read/write namespaced preferences.
    pub preferences: Capability,
    /// Open an HTTP(S) or mail URL through the host.
    pub open_url: Capability,
}

#[derive(Default)]
struct Activations {
    urls: RefCell<VecDeque<url::Url>>,
    changed: event_listener::Event,
}

/// Shared application host services and a bounded URL activation inbox.
#[derive(Clone, Default)]
pub struct Platform {
    activations: Rc<Activations>,
}

impl std::fmt::Debug for Platform {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Platform").finish_non_exhaustive()
    }
}

impl Platform {
    /// Detects current host availability without prompting for permission.
    pub fn capabilities(&self) -> Capabilities {
        #[cfg(target_family = "wasm")]
        return browser::capabilities();

        #[cfg(not(target_family = "wasm"))]
        {
            let ui = Capability {
                supported: cfg!(feature = "gpui"),
                permission: PermissionState::NotRequired,
                requires_user_gesture: false,
            };

            Capabilities {
                file_import: ui,
                file_export: ui,
                clipboard_read: ui,
                clipboard_write: ui,
                open_url: ui,
                preferences: Capability {
                    supported: true,
                    ..ui
                },
            }
        }
    }

    /// Opens an explicitly namespaced preference store. Values use last-writer-wins
    /// replacement; this is not a document database or a multi-key transaction.
    ///
    /// # Errors
    /// Rejects empty/traversal names or an unavailable native application-data root.
    pub fn preferences(&self, namespace: &str) -> Result<Preferences, Error> {
        Preferences::open(namespace)
    }

    /// Queues a validated application URL received from a host or custom launcher.
    /// The oldest activation is discarded beyond 32 pending URLs. This does not
    /// navigate automatically; the application owns scheme/host/argument validation.
    ///
    /// # Errors
    /// Rejects malformed or oversized URLs before queuing them.
    pub fn receive_open_url(&self, value: &str) -> Result<(), Error> {
        if value.len() > 16 * 1024 {
            return Err(Error::InvalidInput("activation URL exceeds 16 KiB".into()));
        }

        let url = url::Url::parse(value).map_err(|error| Error::InvalidInput(error.to_string()))?;
        let mut urls = self.activations.urls.borrow_mut();

        if urls.len() == 32 {
            urls.pop_front();
        }

        urls.push_back(url);
        drop(urls);
        self.activations.changed.notify(usize::MAX);

        Ok(())
    }

    /// Waits for the next activation. Clones share one inbox; competing consumers
    /// divide events. Keep one application-owned consumer and cancel it at shutdown.
    pub async fn next_open_url(&self) -> url::Url {
        loop {
            let listener = self.activations.changed.listen();

            if let Some(url) = self.activations.urls.borrow_mut().pop_front() {
                return url;
            }

            listener.await;
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

#[cfg(any(feature = "gpui", test))]
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
