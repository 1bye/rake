//! Covers: standalone wire decoding, route codecs and legacy inspection/checkpoint defaults.
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

#[test]
fn inspection_origins_and_partial_boxes_roundtrip_without_breaking_old_snapshots() {
    use rake_protocol::inspection::{Element, StyleOrigin};
    use serde_json::json;
    let legacy = json!({"id":"element-1-0","parent":null,"kind":"Div", "metadata_truncated":false,
        "resolved":{"properties":{"opacity":1.0},"hovered":false}});

    let element: Element = serde_json::from_value(legacy.clone()).unwrap();

    assert!(element.box_model.is_none());
    assert!(element.view.is_none());
    assert!(element.text.is_none());
    assert!(!element.text_editable);
    assert!(element.resolved.unwrap().layers.is_empty());

    let mut current = legacy;

    current["text"] = json!("Hello 世界\n🌍");
    current["text_editable"] = json!(true);
    current["view"] = json!({"tag":"section","style_sources":[{"scope":"section","attribute":"class","expression":"\"p-2\"","source":{"file":"app.rs","line":8,"column":4}}]});
    current["box_model"] = json!({
        "border":{"x":1.,"y":2.,"width":100.,"height":40.},
        "padding":null,"content":null,"margin":null,"unavailable":["Used edges unavailable"]
    });
    current["resolved"]["layers"] = json!([{"origin":"default","properties":{"opacity":1.0}}]);
    let element: Element = serde_json::from_value(current).unwrap();

    assert_eq!(element.text.as_deref(), Some("Hello 世界\n🌍"));
    assert_eq!(
        element.resolved.as_ref().unwrap().layers[0].origin,
        StyleOrigin::Default
    );
    assert!(element.box_model.as_ref().unwrap().content.is_none());
    assert_eq!(
        serde_json::from_value::<Element>(serde_json::to_value(&element).unwrap()).unwrap(),
        element
    );
}

#[test]
fn literal_text_edit_commands_are_strict_and_portable() {
    use rake_protocol::{control::Command, inspection::TextEdit};
    let command: Command = serde_json::from_value(serde_json::json!({
        "type":"edit_element_text", "id":"text-1", "generation":"1",
        "edit":{"action":"set","text":"<literal> café 你好 🙂\n"}
    }))
    .unwrap();

    assert!(
        matches!(&command, Command::EditElementText { edit: TextEdit::Set { text }, .. } if text == "<literal> café 你好 🙂\n")
    );
    assert_eq!(
        serde_json::from_value::<Command>(serde_json::to_value(&command).unwrap()).unwrap(),
        command
    );
    assert!(serde_json::from_str::<TextEdit>(r#"{"action":"reset","extra":true}"#).is_err());
    assert!(serde_json::from_str::<TextEdit>(r#"{"action":"set","text":null}"#).is_err());
}
