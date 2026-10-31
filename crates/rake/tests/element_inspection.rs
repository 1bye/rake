//! Covers: headless inspection availability, strict wire queries and runtime scoping.
//! Real: public runtime client, protocol and JSON serialization. Doubles: none.
//! Requires: no GPU, browser or network.

use rake::control::{ControlError, Message, PROTOCOL_VERSION, Query, Reply, Request};
use rake::inspection::{InspectionError, Snapshot, Status};
use rake::router::{NoRoute, Router, RouterConfig};

#[test]
fn headless_queries_report_coverage_without_inventing_a_tree() {
    let router = Router::<NoRoute>::new(RouterConfig::default());
    let client = router.client();

    let request = |query| Request {
        protocol: PROTOCOL_VERSION,
        runtime_id: router.runtime_id(),
        key: None,
        message: Message::Query(query),
    };

    let Reply::Elements(snapshot): Reply = client.request(request(Query::InspectTree {})).unwrap()
    else {
        panic!("tree reply");
    };

    let snapshot: Snapshot =
        serde_json::from_value(serde_json::to_value(snapshot).unwrap()).unwrap();

    assert_eq!(snapshot.status, Status::Unavailable);
    assert!(snapshot.elements.is_empty());
    assert!(!snapshot.coverage.is_empty());
    assert_eq!(client.info().element_inspection, Status::Unavailable);
    assert!(!client.info().element_interaction);
    assert!(matches!(
        client.request(request(Query::InspectPick {})),
        Err(ControlError::Inspection(InspectionError::Unavailable))
    ));
    assert!(matches!(
        client.request(Request {
            protocol: PROTOCOL_VERSION,
            runtime_id: router.runtime_id(),
            key: None,
            message: Message::Command(rake::control::Command::HighlightElement {
                id: None,
                generation: String::new()
            })
        }),
        Err(ControlError::Inspection(InspectionError::Unavailable))
    ));
    assert!(matches!(
        client.request(request(Query::InspectElement {
            id: "element-1-0".into()
        })),
        Err(ControlError::Inspection(InspectionError::Unavailable))
    ));
    let other = Router::<NoRoute>::new(RouterConfig::default());
    let mut stale = request(Query::InspectTree {});

    stale.runtime_id = other.runtime_id();
    assert!(matches!(
        client.request(stale),
        Err(ControlError::WrongRuntime)
    ));
    assert!(
        serde_json::from_str::<Query>(
            r#"{"type":"inspect_element","id":"element-1-0","evaluate":"source"}"#
        )
        .is_err()
    );
}

#[cfg(all(feature = "gpui", feature = "devtools"))]
#[test]
fn manual_handler_metadata_is_available_through_the_public_builder_api() {
    use rake::gpui::{InteractiveElement as _, StatefulInteractiveElement as _};
    use rake::inspection::InspectElement as _;
    let element = rake::gpui::div()
        .id("save")
        .on_click(|_, _, _| {})
        .inspect("save")
        .inspect_handler("click");

    assert!(rake::gpui::Element::id(&element.into_inner()).is_some());
}
