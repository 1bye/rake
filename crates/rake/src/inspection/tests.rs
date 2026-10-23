//! Covers: exact capture lookup, unavailable/waiting states and snapshot replacement.
//! Real: owned store and serialization. Doubles: captured metadata, no GPUI traversal.
//! Requires: none.

use super::*;

#[test]
fn identities_expire_without_semantic_retargeting() {
    let store = Store::default();

    assert!(matches!(
        store.element("element-1-0"),
        Err(InspectionError::Unavailable)
    ));
    store.0.borrow_mut().status = Status::Waiting;
    assert!(matches!(
        store.element("element-1-0"),
        Err(InspectionError::Waiting)
    ));
    let element = Element {
        id: "element-1-0".into(),
        parent: None,
        kind: "Div".into(),
        semantic_id: Some("save".into()),
        source: None,
        layout: None,
        styles: None,
        metadata_truncated: false,
        tab_id: None,
        entry_id: None,
    };

    let mut snapshot = store.0.borrow_mut();

    snapshot.status = Status::Captured;
    snapshot.elements.push(element);
    drop(snapshot);
    let owned = store.element("element-1-0").unwrap();

    store.0.borrow_mut().elements[0].id = "element-2-0".into();
    assert!(matches!(
        store.element("element-1-0"),
        Err(InspectionError::NotRetained)
    ));
    assert!(matches!(
        store.element("save"),
        Err(InspectionError::NotRetained)
    ));
    assert_eq!(owned.id, "element-1-0");
    assert_eq!(
        store.element("element-2-0").unwrap().semantic_id.as_deref(),
        Some("save")
    );
}

#[test]
fn count_and_byte_limits_report_loss_and_fit_the_transport() {
    let store = Store::default();

    let template = Element {
        id: "element-1-0".into(),
        parent: None,
        kind: "Div".into(),
        semantic_id: None,
        source: None,
        layout: None,
        styles: None,
        metadata_truncated: false,
        tab_id: None,
        entry_id: None,
    };

    let mut snapshot = store.snapshot();
    let mut bytes = 0;

    for index in 0..MAX_ELEMENTS + 10 {
        let mut element = template.clone();

        element.id = format!("element-1-{index}");
        snapshot.retain(element, &mut bytes);
    }

    assert_eq!(snapshot.elements.len(), MAX_ELEMENTS);
    assert_eq!(snapshot.omitted, 10);
    assert!(serde_json::to_vec(&snapshot).unwrap().len() < MAX_SNAPSHOT_BYTES);
    let mut snapshot = store.snapshot();
    let mut bytes = 0;
    let mut element = template;

    element.styles = Some(Value::String("\\".repeat(4000)));

    for _ in 0..MAX_ELEMENTS {
        snapshot.retain(element.clone(), &mut bytes);
    }

    assert!(snapshot.elements.len() < MAX_ELEMENTS);
    assert!(snapshot.omitted > 0);
    assert!(serde_json::to_vec(&snapshot).unwrap().len() < MAX_SNAPSHOT_BYTES);
}
