//! Covers: denial, delegation, replacement, dismissal and activation routing.
//! Real: Rake service and URL inbox. Doubles: GPUI TestPlatform and authorization result.
//! Requires: view-test-support; no bundle, display, GPU or permission prompt.

use super::*;
use futures::FutureExt as _;

fn notification(payload: &str) -> Notification {
    Notification {
        tag: "export".into(),
        title: "Export finished".into(),
        body: "Open export".into(),
        activation_url: payload.into(),
    }
}

fn submit(
    platform: &Platform,
    notification: Notification,
    state: PermissionState,
    cx: &gpui::App,
) -> Request<NotificationOutcome> {
    let payload = notification.validate().unwrap();

    platform.submit_native(
        notification,
        payload,
        Box::pin(async move { Ok(state) }),
        cx,
    )
}

#[gpui::test]
async fn denial_does_not_post_or_retain_payload(cx: &mut gpui::TestAppContext) {
    let platform = Platform::default();
    cx.update(|cx| platform.install_notifications("test.notifications", cx));
    let request = cx.update(|cx| {
        submit(
            &platform,
            notification("app://export/1"),
            PermissionState::Denied,
            cx,
        )
    });

    assert_eq!(request.await.unwrap(), NotificationOutcome::Denied);
    assert!(cx.shown_system_notifications().is_empty());
    assert!(platform.notifications.pending.borrow().is_empty());
    assert!(platform.next_open_url().now_or_never().is_none());
}

#[gpui::test]
async fn replacement_click_and_dismissal_use_one_shared_activation_inbox(
    cx: &mut gpui::TestAppContext,
) {
    let platform = Platform::default();
    let consumer = platform.clone();
    cx.update(|cx| platform.install_notifications("test.notifications", cx));

    for payload in ["app://export/1", "app://export/2"] {
        let request = cx.update(|cx| {
            submit(
                &platform,
                notification(payload),
                PermissionState::Granted,
                cx,
            )
        });
        assert_eq!(request.await.unwrap(), NotificationOutcome::Submitted);
    }

    assert_eq!(cx.delivered_system_notifications().len(), 1);
    assert_eq!(cx.app_identity().unwrap().0.as_ref(), "test.notifications");
    platform.receive_open_url("app://external/0").unwrap();
    cx.simulate_system_notification_response(gpui::SystemNotificationResponse {
        tag: "unrelated".into(),
        action_id: None,
    });
    cx.simulate_system_notification_response(gpui::SystemNotificationResponse {
        tag: "rake.notification:export".into(),
        action_id: Some("unsupported-action".into()),
    });
    cx.simulate_system_notification_response(gpui::SystemNotificationResponse {
        tag: "rake.notification:export".into(),
        action_id: None,
    });

    assert_eq!(consumer.next_open_url().await.as_str(), "app://external/0");
    assert_eq!(consumer.next_open_url().await.as_str(), "app://export/2");
    assert!(cx.delivered_system_notifications().is_empty());
    assert!(platform.notifications.pending.borrow().is_empty());
    cx.simulate_system_notification_response(gpui::SystemNotificationResponse {
        tag: "rake.notification:export".into(),
        action_id: None,
    });
    assert!(consumer.next_open_url().now_or_never().is_none());
}

#[gpui::test]
async fn dismissal_forgets_payload_even_if_late_response_arrives(cx: &mut gpui::TestAppContext) {
    let platform = Platform::default();
    cx.update(|cx| platform.install_notifications("test.notifications", cx));
    let request = cx.update(|cx| {
        submit(
            &platform,
            notification("app://export/1"),
            PermissionState::Unknown,
            cx,
        )
    });
    request.await.unwrap();

    // Exercise the callback even on unpackaged macOS, where public dismissal is unavailable.
    cx.update(|cx| {
        platform.dismiss_bound_notification("export", cx);
    });
    cx.simulate_system_notification_response(gpui::SystemNotificationResponse {
        tag: "rake.notification:export".into(),
        action_id: None,
    });

    assert!(cx.delivered_system_notifications().is_empty());
    assert!(platform.next_open_url().now_or_never().is_none());
}

#[cfg(target_os = "macos")]
#[gpui::test]
async fn unpackaged_macos_returns_unsupported_without_prompt_or_post(
    cx: &mut gpui::TestAppContext,
) {
    assert!(
        !macos::available(),
        "run this headless test executable outside an app bundle"
    );
    let platform = Platform::default();
    cx.update(|cx| platform.install_notifications("test.notifications", cx));
    let request = cx.update(|cx| platform.show_notification(notification("app://export/1"), cx));

    assert!(matches!(request.await, Err(Error::Unsupported(_))));
    assert!(matches!(
        platform.notification_permission().await,
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        platform.request_notification_permission().await,
        Err(Error::Unsupported(_))
    ));
    assert!(cx.shown_system_notifications().is_empty());
}
