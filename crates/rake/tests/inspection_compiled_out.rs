//! Covers: production expansion, original element types, lazy names and absent hosts.
//! Real: public facade/macro, GPUI element construction and runtime capabilities.
//! Doubles: none. Requires: experimental-view without devtools; no GPU or display.

#![cfg(all(feature = "experimental-view", not(feature = "devtools")))]

use rake::gpui::{self, IntoElement};
use rake::inspection::InspectElement as _;

#[derive(gpui::IntoElement)]
struct Panel {
    id: String,
}

impl gpui::RenderOnce for Panel {
    fn render(self, _: &mut gpui::Window, _: &mut gpui::App) -> impl IntoElement {
        rake::view! { <div id={self.id}/> }
    }
}

#[test]
fn markup_returns_original_types_and_removes_metadata_tokens() {
    let _: gpui::Stateful<gpui::Div> =
        rake::view! { <div class="p-2"><p>Text</p>{"Accessible text"}</div> };

    let _: Panel = rake::view! { <Panel id={"owned".to_owned()}/> };

    let _: gpui::Div = rake::__private::inspection!(
        compile_error!("inspection-only code must not expand in this build"),
        gpui::div()
    );

    let _: gpui::Div =
        gpui::div().inspect_with(|| panic!("production must not construct inspection names"));

    let _: gpui::Div = rake::inspection::inspect(gpui::div());
}

#[test]
fn production_capabilities_cannot_enable_inspection_or_edits() {
    use rake::control::{Command, ControlError, Message, PROTOCOL_VERSION, Query, Reply, Request};
    use rake::inspection::{InspectionError, Status, StyleEdit};
    use rake::router::{NoRoute, Router, RouterConfig};
    let router = Router::<NoRoute>::new(RouterConfig::default());
    let client = router.client();

    assert_eq!(client.info().element_inspection, Status::Unavailable);
    assert!(!client.info().element_interaction);
    assert!(
        !client
            .info()
            .commands
            .iter()
            .any(|name| name.starts_with("preview"))
    );
    assert!(
        !client
            .info()
            .queries
            .iter()
            .any(|name| name.starts_with("preview"))
    );
    let request = |message| Request {
        protocol: PROTOCOL_VERSION,
        runtime_id: router.runtime_id(),
        key: None,
        message,
    };

    let Reply::Elements(tree) = client
        .request(request(Message::Query(Query::InspectTree {})))
        .unwrap()
    else {
        panic!("expected tree");
    };

    assert!(tree.elements.is_empty());
    assert!(tree.coverage.contains("compiled out"));

    for command in [
        Command::PreviewInput {
            point: rake::inspection::preview::Point {
                window: rake::router::WindowId(1),
                native_id: 42,
                generation: "0".into(),
                geometry: None,
                x: 10,
                y: 20,
            },
            input: rake::inspection::preview::Input::Click,
        },
        Command::InspectMode {
            enabled: true,
            lease_ms: 1000,
        },
        Command::EditElementText {
            id: "anything".into(),
            generation: "0".into(),
            edit: rake::inspection::TextEdit::Set {
                text: "replacement".into(),
            },
        },
        Command::EditElementStyle {
            id: "anything".into(),
            generation: "0".into(),
            edit: StyleEdit::Reset,
        },
    ] {
        assert!(matches!(
            client.request(request(Message::Command(command))),
            Err(ControlError::Inspection(InspectionError::Unavailable))
        ));
    }
}
