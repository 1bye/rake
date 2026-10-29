//! Covers: standalone wire decoding, route codecs and legacy checkpoint defaults.
//! Real: protocol contracts and Serde. Doubles: none. Requires: none.

use rake_protocol::control::{Message, Query, Request};
use rake_protocol::router::{RouteAddress, WindowId};

#[test]
fn requests_decode_without_loading_a_runtime_or_ui() {
    let request: Request = serde_json::from_value(serde_json::json!({
        "protocol": 1, "runtime_id": "00000000-0000-4000-8000-000000000001", "key": null,
        "message": {"type": "query", "value": {"type": "tasks", "owner": "Application"}}
    }))
    .unwrap();

    assert!(matches!(
        request.message,
        Message::Query(Query::Tasks {
            owner: Some(rake_protocol::tasks::TaskOwner::Application)
        })
    ));
    assert!(serde_json::from_str::<Query>(r#"{"type":"tabs","extra":true}"#).is_err());
    let address = RouteAddress {
        route: "document".into(),
        params: [("id".into(), "one/two".into())].into(),
    };

    assert_eq!(
        serde_json::from_value::<RouteAddress>(serde_json::to_value(&address).unwrap()).unwrap(),
        address
    );
}

#[test]
fn omitted_window_defaults_keep_legacy_checkpoint_compatibility() {
    let tab: rake_protocol::reload::TabCheckpoint = serde_json::from_value(serde_json::json!({
        "id": 1, "mode": "Foreground", "cursor": 0, "selected_at": 0, "entries": []
    }))
    .unwrap();

    assert_eq!(tab.window_id, "window-1".parse::<WindowId>().unwrap());
}
