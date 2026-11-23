//! Notification contracts and permissions in headless hosts.

use super::{Capability, Error, PermissionState, Platform, Request};

/// Notification with an application URL delivered when its body is clicked.
/// Tags replace payloads and, where supported, earlier host notifications. Payloads are
/// kept in memory, so activation after process exit is not supported.
///
/// ```
/// # use rake_core as rake;
/// use rake::platform::Notification;
/// let notification = Notification {
///     tag: "export-42".into(),
///     title: "Export finished".into(),
///     body: "Open the exported document".into(),
///     activation_url: "myapp://exports/42".into(),
/// };
/// assert_eq!(notification.tag, "export-42");
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    /// Stable nonempty identifier, at most 128 UTF-8 bytes.
    pub tag: String,
    /// Nonempty headline, at most 1 KiB of UTF-8 text.
    pub title: String,
    /// Additional text, at most 16 KiB of UTF-8 text; rendering follows host policy.
    pub body: String,
    /// Absolute application URL, at most 16 KiB; validate its arguments on receipt.
    pub activation_url: String,
}

impl Notification {
    #[doc(hidden)]
    pub fn validate(&self) -> Result<url::Url, Error> {
        if self.tag.is_empty()
            || self.tag.len() > 128
            || self.title.is_empty()
            || self.title.len() > 1024
            || self.body.len() > 16 * 1024
        {
            return Err(Error::InvalidInput(
                "notification tag/title/body exceeds its limits".into(),
            ));
        }

        super::activation_url(&self.activation_url)
    }
}

/// Delegation is distinct from permission denial and actual on-screen delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationOutcome {
    /// Submitted to the host; focus modes and host policy may suppress delivery.
    Submitted,
    /// Permission is absent. The application can continue or request authorization.
    Denied,
}

#[cfg(target_family = "wasm")]
mod browser;

pub(super) fn capability() -> Capability {
    #[cfg(target_family = "wasm")]
    return browser::capability();

    #[cfg(not(target_family = "wasm"))]
    Capability {
        supported: false,
        permission: PermissionState::Unknown,
        requires_user_gesture: false,
    }
}

impl Platform {
    /// Queries notification permission without prompting.
    pub fn notification_permission(&self) -> Request<PermissionState> {
        #[cfg(target_family = "wasm")]
        return browser::permission(false);

        #[cfg(not(target_family = "wasm"))]
        Box::pin(async { Err(Error::Unsupported("notifications")) })
    }

    /// Requests notification permission when the host supports it.
    pub fn request_notification_permission(&self) -> Request<PermissionState> {
        #[cfg(target_family = "wasm")]
        return browser::permission(true);

        #[cfg(not(target_family = "wasm"))]
        self.notification_permission()
    }
}
