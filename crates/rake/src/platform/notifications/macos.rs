//! Authorization only; delivery and response ownership stay with GPUI.

use super::{Error, PermissionState, Request};
use block2::RcBlock;
use futures::channel::oneshot;
use objc2::runtime::Bool;
use objc2_foundation::{NSBundle, NSError};

use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNUserNotificationCenter,
};

use std::sync::Mutex;

pub(super) fn available() -> bool {
    // The notification center aborts an unpackaged process before returning an error.
    NSBundle::mainBundle().bundleIdentifier().is_some()
}

pub(super) fn permission(prompt: bool) -> Request<PermissionState> {
    if !available() {
        return Box::pin(async {
            Err(Error::Unsupported(
                "notifications outside a macOS app bundle",
            ))
        });
    }

    let center = UNUserNotificationCenter::currentNotificationCenter();
    let (sender, receiver) = oneshot::channel();
    let sender = Mutex::new(Some(sender));

    if prompt {
        let completion = RcBlock::new(move |granted: Bool, error: *mut NSError| {
            // SAFETY: the framework supplies a valid NSError for the callback duration, or null.
            let result = if let Some(error) = unsafe { error.as_ref() } {
                Err(Error::Unavailable {
                    operation: "notification authorization",
                    message: error.localizedDescription().to_string(),
                })
            } else if granted.as_bool() {
                Ok(PermissionState::Granted)
            } else {
                Ok(PermissionState::Denied)
            };

            if let Some(sender) = sender
                .lock()
                .expect("notification authorization lock poisoned")
                .take()
            {
                let _ = sender.send(result);
            }
        });

        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
            &completion,
        );
    } else {
        let completion = RcBlock::new(
            move |settings: std::ptr::NonNull<objc2_user_notifications::UNNotificationSettings>| {
                // SAFETY: the framework supplies non-null settings valid for this callback.
                let status = unsafe { settings.as_ref() }.authorizationStatus();

                let permission = match status {
                    UNAuthorizationStatus::NotDetermined => PermissionState::Prompt,
                    UNAuthorizationStatus::Denied => PermissionState::Denied,
                    UNAuthorizationStatus::Authorized
                    | UNAuthorizationStatus::Provisional
                    | UNAuthorizationStatus::Ephemeral => PermissionState::Granted,
                    _ => PermissionState::Unknown,
                };

                if let Some(sender) = sender
                    .lock()
                    .expect("notification settings lock poisoned")
                    .take()
                {
                    let _ = sender.send(Ok(permission));
                }
            },
        );

        center.getNotificationSettingsWithCompletionHandler(&completion);
    }

    Box::pin(async move {
        receiver.await.map_err(|_| Error::Unavailable {
            operation: "notification authorization",
            message: "authorization callback was dropped".into(),
        })?
    })
}
