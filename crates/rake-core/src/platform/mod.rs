//! Portable host services with explicit support, cancellation and permission boundaries.
//!
//! Obtain the shared handle from [`crate::Services::platform`]. Picker and clipboard
//! methods initiate requests immediately, before their returned future is polled;
//! call them in a user event, then await in owned work. Dropping a request stops
//! observation, but cannot undo an OS dialog, clipboard write or browser download.

mod notifications;
mod preferences;
mod secrets;
mod shortcuts;

pub use notifications::{Notification, NotificationOutcome};
pub use preferences::Preferences;
pub use secrets::{Secret, Secrets};
pub use shortcuts::GlobalShortcuts;

#[cfg(target_family = "wasm")]
pub mod browser;

use std::cell::{OnceCell, RefCell};
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

/// An initiated host request; browser work may require the initiating user event.
pub type Request<T> = Pin<Box<dyn Future<Output = Result<T, Error>>>>;

/// A host operation failed without silently substituting another behavior.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No application id was configured for the credential namespace.
    #[error("secrets require an application id configured through App::new")]
    MissingApplicationId,
    /// No Linux Secret Service is registered on the session bus.
    #[error("no Secret Service is available")]
    NoSecretService,
    /// Credential storage is absent or failed; backend messages are withheld.
    #[error("system credential storage is unavailable")]
    SecretsUnavailable,
    /// Keychain disallows interaction, commonly because it is locked.
    #[error("keychain is locked or interaction is unavailable")]
    KeychainLocked,
    /// Credential access was denied or cancelled.
    #[error("credential access denied")]
    SecretAccessDenied,
    /// Windows Credential Manager accepts at most 2560 bytes.
    #[error("secret exceeds the Windows 2560-byte limit")]
    SecretSizeLimit,
    /// No implementation exists in the current host.
    #[error("{0} is unsupported by this host")]
    Unsupported(&'static str),
    /// Another application or registration owns the requested shortcut.
    #[error("global shortcut is already registered")]
    ShortcutConflict,
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
    /// Post OS notifications; permission and delivery remain separate.
    pub notifications: Capability,
    /// Register a system-wide shortcut; headless and Web are unsupported.
    pub global_shortcuts: Capability,
}

#[derive(Default)]
struct Activations {
    urls: RefCell<VecDeque<url::Url>>,
    changed: event_listener::Event,
}

impl Activations {
    fn receive(&self, value: &str) -> Result<(), Error> {
        let url = activation_url(value)?;
        let mut urls = self.urls.borrow_mut();

        if urls.len() == 32 {
            urls.pop_front();
        }

        urls.push_back(url);
        drop(urls);
        self.changed.notify(usize::MAX);

        Ok(())
    }
}

/// Shared application host services and a bounded URL activation inbox.
#[derive(Clone, Default)]
pub struct Platform {
    application_id: Option<Rc<str>>,
    activations: Rc<Activations>,
    shortcuts: GlobalShortcuts,
    host: Rc<OnceCell<Box<dyn std::any::Any>>>,
}

impl std::fmt::Debug for Platform {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Platform").finish_non_exhaustive()
    }
}

impl Platform {
    #[doc(hidden)]
    pub fn for_application(application_id: &str) -> Self {
        Self {
            application_id: Some(application_id.into()),
            ..Self::default()
        }
    }

    /// Detects current host availability without prompting for permission.
    pub fn capabilities(&self) -> Capabilities {
        #[cfg(target_family = "wasm")]
        return browser::capabilities();

        #[cfg(not(target_family = "wasm"))]
        {
            let ui = Capability {
                supported: false,
                permission: PermissionState::NotRequired,
                requires_user_gesture: false,
            };

            Capabilities {
                notifications: notifications::capability(),
                global_shortcuts: self.shortcuts.capability(),
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
        self.activations.receive(value)
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

    #[cfg(target_family = "wasm")]
    #[doc(hidden)]
    pub fn open_url_receiver(&self) -> impl Fn(&str) + use<> {
        let activations = Rc::downgrade(&self.activations);

        move |value| {
            if let Some(activations) = activations.upgrade() {
                let _ = activations.receive(value);
            }
        }
    }

    #[doc(hidden)]
    pub fn host_state<T: 'static>(&self, init: impl FnOnce() -> T) -> &T {
        self.host
            .get_or_init(|| Box::new(init()))
            .downcast_ref()
            .expect("a platform has one host state type")
    }
}

fn activation_url(value: &str) -> Result<url::Url, Error> {
    if value.len() > 16 * 1024 {
        return Err(Error::InvalidInput("activation URL exceeds 16 KiB".into()));
    }

    let url = url::Url::parse(value).map_err(|error| Error::InvalidInput(error.to_string()))?;

    if url.as_str().len() > 16 * 1024 {
        return Err(Error::InvalidInput(
            "encoded activation URL exceeds 16 KiB".into(),
        ));
    }

    Ok(url)
}
