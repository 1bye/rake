//! Covers: standalone capabilities, identity validation, diagnostics, inspection controls and teardown.
//! Real: inspection store, collector, protocol and Unix transport. Doubles: owned capture fixture.
//! Requires: local Unix sockets for the transport test; no GUI/GPU.

use super::*;
use crate::control::host::Host;

fn request(client: &InspectionClient, message: Message) -> Request {
    Request {
        protocol: PROTOCOL_VERSION,
        runtime_id: client.info().runtime_id,
        key: None,
        message,
    }
}

#[test]
fn standalone_capabilities_and_validation_do_not_advertise_or_accept_router_actions() {
    let client = InspectionClient::new(Collector::default());
    let info = client.info();
    assert_eq!(info.element_inspection, Status::Waiting);
    assert!(info.element_interaction);
    assert!(info.supported_targets.is_empty());
    assert_eq!(info.idempotency_capacity, 0);
    assert!(
        !info
            .queries
            .iter()
            .any(|q| ["events", "routes", "snapshot", "reload"].contains(&q.as_str()))
    );
    assert_eq!(
        info.commands,
        [
            "navigate_element",
            "highlight_element",
            "inspect_mode",
            "edit_element_style",
            "record_frames"
        ]
    );
    assert!(matches!(
        client.request(request(&client, Message::Query(Query::Snapshot {}))),
        Err(ControlError::Unsupported { .. })
    ));
    assert!(matches!(
        client.request(request(
            &client,
            Message::Command(Command::Quit {
                options: Default::default()
            })
        )),
        Err(ControlError::Unsupported { .. })
    ));
    let mut wrong = request(&client, Message::Query(Query::Info {}));
    wrong.protocol += 1;
    assert!(matches!(
        client.request(wrong),
        Err(ControlError::ProtocolVersion { .. })
    ));
    let other = InspectionClient::new(Collector::default());
    assert!(matches!(
        client.request(request(&other, Message::Query(Query::Info {}))),
        Err(ControlError::WrongRuntime)
    ));
    client.close();
    assert!(matches!(
        client.request(request(&client, Message::Query(Query::Info {}))),
        Err(ControlError::RuntimeShutdown)
    ));
}

#[test]
fn inspection_leases_and_diagnostics_use_the_existing_shared_contracts() {
    let collector = Collector::default();
    collector.record(
        "INFO",
        "standalone",
        "event",
        Vec::new(),
        [("message".into(), serde_json::json!("ready"))].into(),
    );
    let client = InspectionClient::new(collector);
    let snapshot = client.inspection().begin(0);
    client.inspection().publish(WindowId(1), snapshot);
    let Reply::Diagnostics(diagnostics) = client
        .request(request(
            &client,
            Message::Query(Query::Diagnostics {
                after: 0,
                kind: None,
            }),
        ))
        .unwrap()
    else {
        panic!("diagnostics expected")
    };
    assert_eq!(diagnostics.records.len(), 1);
    assert!(
        matches!(client.request(request(&client, Message::Query(Query::InspectTree {}))).unwrap(), Reply::Elements(snapshot) if snapshot.status == Status::Captured)
    );
    for _ in 0..100 {
        assert!(matches!(
            client
                .request(request(
                    &client,
                    Message::Command(Command::InspectMode {
                        enabled: true,
                        lease_ms: 100
                    })
                ))
                .unwrap(),
            Reply::Acknowledged
        ));
    }
    assert!(
        matches!(client.request(request(&client, Message::Query(Query::InspectPick {}))).unwrap(), Reply::InspectPick(pick) if pick.mode)
    );
    client
        .request(request(
            &client,
            Message::Command(Command::InspectMode {
                enabled: false,
                lease_ms: 0,
            }),
        ))
        .unwrap();
    assert!(
        matches!(client.request(request(&client, Message::Query(Query::InspectPick {}))).unwrap(), Reply::InspectPick(pick) if !pick.mode)
    );
    client.close();
    assert_eq!(client.info().element_inspection, Status::Unavailable);
    assert!(!client.inspection().pick().unwrap().mode);
}

#[cfg(unix)]
#[test]
fn unix_transport_discovers_dispatches_and_removes_standalone_endpoints() {
    use crate::control::local::{self, LocalServer};
    let directory =
        std::path::PathBuf::from("/tmp").join(format!("rake-root-{}", uuid::Uuid::new_v4()));
    let client = InspectionClient::new(Collector::default());
    let host = client.host();
    let server = LocalServer::bind_host(&host, "standalone.fixture", &directory).unwrap();
    let instances = local::discover(&directory).unwrap();
    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0].info.runtime_id, client.info().runtime_id);
    let local::Endpoint::Local { path } = instances[0].endpoint.clone() else {
        panic!("native endpoint expected")
    };
    let request = request(&client, Message::Query(Query::InspectTree {}));
    let reader = std::thread::spawn(move || {
        local::exchange(&path, &request, std::time::Duration::from_secs(5)).unwrap()
    });
    let work = async {
        loop {
            if reader.is_finished() {
                break;
            }
            crate::http::sleep(std::time::Duration::from_millis(1)).await;
        }
    };
    futures::executor::block_on(futures::future::select(
        Box::pin(server.run_host(host)),
        Box::pin(work),
    ));
    assert!(matches!(
        reader.join().unwrap().unwrap(),
        Reply::Elements(_)
    ));
    drop(server);
    assert!(std::fs::read_dir(&directory).unwrap().next().is_none());
    std::fs::remove_dir(directory).unwrap();
}
