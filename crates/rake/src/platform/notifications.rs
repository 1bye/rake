//! Application-owned OS notifications and URL activation payloads.

use super::{Capability, Error, PermissionState, Platform, Request};
#[cfg(feature = "gpui")]
use std::cell::Cell;
#[cfg(any(feature = "gpui", test))]
use std::cell::RefCell;
#[cfg(any(feature = "gpui", test))]
use std::collections::BTreeMap;

#[cfg(target_family = "wasm")]
mod browser;
#[cfg(all(feature = "gpui", target_os = "macos"))]
mod macos;

#[cfg(any(feature = "gpui", test))]
const MAX_PENDING: usize = 128;
#[cfg(all(feature = "gpui", not(target_family = "wasm")))]
const TAG_PREFIX: &str = "rake.notification:";

/// Notification with an application URL delivered when its body is clicked.
/// Tags replace payloads and, where supported, earlier host notifications. Payloads are
/// kept in memory, so activation after process exit is not supported.
///
/// ```
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

#[cfg(any(feature = "gpui", test))]
impl Notification {
    fn validate(&self) -> Result<url::Url, Error> {
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

#[cfg(any(feature = "gpui", test))]
#[derive(Default)]
pub(super) struct Notifications {
    #[cfg(feature = "gpui")]
    installed: Cell<bool>,
    pending: RefCell<BTreeMap<String, url::Url>>,
    #[cfg(all(feature = "gpui", target_family = "wasm"))]
    browser: RefCell<Option<browser::Host>>,
}

pub(super) fn capability() -> Capability {
    #[cfg(target_family = "wasm")]
    return browser::capability();

    #[cfg(not(target_family = "wasm"))]
    Capability {
        supported: cfg!(feature = "gpui") && native_available(),
        permission: PermissionState::Unknown,
        requires_user_gesture: false,
    }
}

#[cfg(not(target_family = "wasm"))]
fn native_available() -> bool {
    #[cfg(all(feature = "gpui", target_os = "macos"))]
    return macos::available();

    #[cfg(not(all(feature = "gpui", target_os = "macos")))]
    cfg!(any(target_os = "linux", target_os = "windows"))
}

impl Platform {
    /// Queries authorization without prompting. macOS requires a real app bundle;
    /// Web requires a secure context and the Notification constructor. Native
    /// Windows/Linux return `Unknown`: the delivery API exposes no permission query.
    ///
    /// # Errors
    /// Returns `Unsupported` for headless/unsupported hosts or unpackaged macOS,
    /// and `Unavailable` if the authorization service fails.
    pub fn notification_permission(&self) -> Request<PermissionState> {
        #[cfg(target_family = "wasm")]
        return browser::permission(false);

        #[cfg(all(feature = "gpui", target_os = "macos"))]
        return macos::permission(false);

        #[cfg(not(any(target_family = "wasm", all(feature = "gpui", target_os = "macos"))))]
        Box::pin(async {
            if capability().supported {
                Ok(PermissionState::Unknown)
            } else {
                Err(Error::Unsupported("notifications"))
            }
        })
    }

    /// Initiates authorization immediately. Call directly from a user event on Web,
    /// then await the returned request in owned work. Denial is `Ok(Denied)`;
    /// repeated calls respect the host's existing decision. Never prompts at startup.
    /// Native Windows/Linux return `Unknown` without showing a dialog.
    ///
    /// # Errors
    /// Returns the same errors as [`Self::notification_permission`], plus browser
    /// security policy/user-gesture errors.
    pub fn request_notification_permission(&self) -> Request<PermissionState> {
        #[cfg(target_family = "wasm")]
        return browser::permission(true);

        #[cfg(all(feature = "gpui", target_os = "macos"))]
        return macos::permission(true);

        #[cfg(not(any(target_family = "wasm", all(feature = "gpui", target_os = "macos"))))]
        self.notification_permission()
    }

    /// Posts a notification and retains a URL payload for the shared activation inbox.
    /// Web never prompts here: request authorization from a user event first.
    /// macOS authorization is initiated immediately and awaited before submission.
    /// Keep one consumer of [`Self::next_open_url`] to validate and route payloads
    /// or dispatch application commands. At most 128 distinct tags may be pending;
    /// clicks and explicit dismissal release them. Dropping the request does not
    /// revoke a permission prompt or a notification already submitted. Await in
    /// application-owned foreground work while the GPUI application is alive.
    ///
    /// # Errors
    /// Returns `InvalidInput` for malformed URLs/text limits, `Unsupported` when
    /// unavailable, and `Unavailable` for missing startup binding, capacity or host
    /// failure. Permission denial returns [`NotificationOutcome::Denied`].
    #[cfg(feature = "gpui")]
    pub fn show_notification(
        &self,
        notification: Notification,
        cx: &gpui::App,
    ) -> Request<NotificationOutcome> {
        let payload = match notification.validate() {
            Ok(payload) => payload,
            Err(error) => return Box::pin(async { Err(error) }),
        };

        if !self.notifications.installed.get() || !capability().supported {
            return Box::pin(async {
                Err(Error::Unsupported(
                    "notifications (requires an application host)",
                ))
            });
        }

        #[cfg(target_family = "wasm")]
        {
            let _ = cx;
            let result = browser::show(self, notification, payload);

            Box::pin(async { result })
        }

        #[cfg(not(target_family = "wasm"))]
        {
            self.submit_native(
                notification,
                payload,
                self.request_notification_permission(),
                cx,
            )
        }
    }

    #[cfg(all(feature = "gpui", not(target_family = "wasm")))]
    fn submit_native(
        &self,
        notification: Notification,
        payload: url::Url,
        permission: Request<PermissionState>,
        cx: &gpui::App,
    ) -> Request<NotificationOutcome> {
        let platform = self.clone();
        let app = cx.to_async();

        Box::pin(async move {
            if permission.await? == PermissionState::Denied {
                return Ok(NotificationOutcome::Denied);
            }

            app.update(|cx| {
                platform.remember(&notification.tag, payload)?;
                cx.show_system_notification(gpui::SystemNotification {
                    tag: format!("{TAG_PREFIX}{}", notification.tag).into(),
                    title: notification.title.into(),
                    body: notification.body.into(),
                    actions: Vec::new(),
                });

                Ok(NotificationOutcome::Submitted)
            })
        })
    }

    /// Best-effort removal of a pending/delivered notification and its payload.
    /// A missing tag succeeds. Host notification centers may retain their own history.
    ///
    /// # Errors
    /// Returns `Unsupported` if the application notification host is unavailable.
    #[cfg(feature = "gpui")]
    pub fn dismiss_notification(&self, tag: &str, cx: &gpui::App) -> Result<(), Error> {
        if !self.notifications.installed.get() || !capability().supported {
            return Err(Error::Unsupported("notifications"));
        }

        self.dismiss_bound_notification(tag, cx);

        Ok(())
    }

    #[cfg(feature = "gpui")]
    fn dismiss_bound_notification(&self, tag: &str, cx: &gpui::App) {
        self.notifications.pending.borrow_mut().remove(tag);

        #[cfg(target_family = "wasm")]
        {
            let _ = cx;

            browser::dismiss_tag(self, tag);
        }

        #[cfg(not(target_family = "wasm"))]
        cx.dismiss_system_notification(&format!("{TAG_PREFIX}{tag}"));
    }

    #[cfg(any(feature = "gpui", test))]
    fn remember(&self, tag: &str, payload: url::Url) -> Result<(), Error> {
        let mut pending = self.notifications.pending.borrow_mut();

        if pending.len() == MAX_PENDING && !pending.contains_key(tag) {
            return Err(Error::Unavailable {
                operation: "notifications",
                message: "128 notifications are pending; dismiss one before posting another".into(),
            });
        }

        pending.insert(tag.into(), payload);

        Ok(())
    }

    #[cfg(all(feature = "gpui", not(target_family = "wasm")))]
    fn notification_response(&self, tag: &str) -> bool {
        let Some(tag) = tag.strip_prefix(TAG_PREFIX) else {
            return false;
        };

        let payload = self.notifications.pending.borrow_mut().remove(tag);

        if let Some(payload) = payload {
            // The payload was validated before posting; use the same bounded inbox.
            let _ = self.receive_open_url(payload.as_str());

            return true;
        }

        false
    }

    #[cfg(feature = "gpui")]
    pub(crate) fn install_notifications(&self, identifier: &str, cx: &gpui::App) {
        if self.notifications.installed.replace(true) {
            return;
        }

        #[cfg(not(target_family = "wasm"))]
        {
            cx.set_app_identity(identifier, identifier);
            let platform = self.clone();

            cx.on_system_notification_response(move |response, cx| {
                if response.action_id.is_none() && platform.notification_response(&response.tag) {
                    cx.dismiss_system_notification(&response.tag);
                    cx.activate(true);
                }
            });
        }

        #[cfg(target_family = "wasm")]
        {
            let _ = (identifier, cx);

            *self.notifications.browser.borrow_mut() = Some(browser::install(self));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_payload_is_rejected_before_touching_host() {
        let mut notification = Notification {
            tag: "job".into(),
            title: "Done".into(),
            body: String::new(),
            activation_url: "invalid".into(),
        };

        assert!(matches!(
            notification.validate(),
            Err(Error::InvalidInput(_))
        ));
        notification.activation_url = "app://jobs/42".into();
        assert!(notification.validate().is_ok());
        notification.tag.clear();
        assert!(notification.validate().is_err());
    }

    #[test]
    fn oversized_encoded_payload_and_text_are_rejected() {
        let mut notification = Notification {
            tag: "job".into(),
            title: "Done".into(),
            body: String::new(),
            activation_url: format!("app://job/{}", "漢".repeat(2000)),
        };

        assert!(notification.activation_url.len() < 16 * 1024);
        assert!(notification.validate().is_err());
        notification.activation_url = "app://job/42".into();
        notification.body = "a".repeat(16 * 1024);
        assert!(notification.validate().is_ok());
        notification.body.push('a');
        assert!(notification.validate().is_err());
        notification.body.clear();
        notification.title = "a".repeat(1025);
        assert!(notification.validate().is_err());
        notification.title = "Done".into();
        notification.tag = "a".repeat(129);
        assert!(notification.validate().is_err());
    }

    #[test]
    fn capacity_preserves_existing_payloads_and_allows_replacement() {
        let platform = Platform::default();
        let payload = url::Url::parse("app://job/42").unwrap();

        for n in 0..MAX_PENDING {
            platform.remember(&n.to_string(), payload.clone()).unwrap();
        }

        assert!(platform.remember("overflow", payload.clone()).is_err());
        assert!(platform.remember("0", payload).is_ok());
        assert_eq!(platform.notifications.pending.borrow().len(), MAX_PENDING);
    }
}

#[cfg(all(test, feature = "view-test-support", not(target_family = "wasm")))]
#[path = "notifications/tests.rs"]
mod host_tests;
