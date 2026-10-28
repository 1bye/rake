//! Covers: exact capture lookup, unavailable/waiting states and snapshot replacement.
//! Real: owned store and serialization. Doubles: captured metadata, no GPUI traversal.
//! Requires: none.

use super::*;

#[test]
fn display_names_hide_wrappers_paths_and_generic_arguments() {
    for (kind, expected) in [
        ("gpui::elements::div::Div", "Div"),
        (
            "gpui::elements::div::Stateful<gpui::elements::div::Div>",
            "Div",
        ),
        (
            "gpui::view::ViewElement<rake::router::TabBar<app::Destination>>",
            "TabBar",
        ),
        ("gpui::ViewElement<gpui::Stateful<app::Button>>", "Button"),
        ("app::List<app::Item>", "List"),
        ("Button", "Button"),
        ("", ""),
        ("gpui::ViewElement<app::Truncated", "ViewElement"),
    ] {
        assert_eq!(display_name(kind), expected, "{kind}");
    }
}

#[test]
fn identities_expire_without_semantic_retargeting() {
    let store = Store::default();

    assert!(matches!(
        store.element("element-1-0"),
        Err(InspectionError::Unavailable)
    ));
    store.0.borrow_mut().latest.status = Status::Waiting;
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
        content_mask: None,
        styles: None,
        resolved: None,
        metadata_truncated: false,
        tab_id: None,
        entry_id: None,
    };

    let mut state = store.0.borrow_mut();
    let snapshot = &mut state.latest;

    snapshot.status = Status::Captured;
    snapshot.elements.push(element);
    drop(state);
    let owned = store.element("element-1-0").unwrap();

    store.0.borrow_mut().latest.elements[0].id = "element-2-0".into();
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
        content_mask: None,
        styles: None,
        resolved: None,
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

fn boundary(id: &str, parent: Option<&str>, bounds: Layout, mask: Layout) -> Element {
    Element {
        id: id.into(),
        parent: parent.map(str::to_owned),
        kind: "Div".into(),
        semantic_id: None,
        source: None,
        layout: Some(bounds),
        content_mask: Some(mask),
        styles: None,
        resolved: None,
        metadata_truncated: false,
        tab_id: None,
        entry_id: None,
    }
}

#[test]
fn picking_respects_nesting_clipping_order_and_uninstrumented_gaps() {
    let store = Store::default();
    let window = WindowId(1);
    let mut snapshot = store.begin(1);

    let outer = Layout {
        x: 0.,
        y: 0.,
        width: 100.,
        height: 100.,
    };

    let child = Layout {
        x: 10.,
        y: 10.,
        width: 50.,
        height: 50.,
    };

    let clip = Layout {
        x: 10.,
        y: 10.,
        width: 10.,
        height: 10.,
    };

    snapshot.elements = vec![
        boundary("element-1-0", None, outer, outer),
        boundary("element-1-1", Some("element-1-0"), child, clip),
        boundary("element-1-2", Some("element-1-0"), clip, clip),
    ];
    store.publish(window, snapshot);
    store.inspect_mode(true, 1000).unwrap();
    assert!(store.pointer(window, 15., 15., false));
    assert_eq!(
        store.pick().unwrap().hovered.as_deref(),
        Some("element-1-2")
    );
    store.pointer(window, 30., 30., false);
    assert_eq!(
        store.pick().unwrap().hovered.as_deref(),
        Some("element-1-0")
    );
    store.pointer(window, 110., 110., false);
    assert!(store.pick().unwrap().hovered.is_none());
    let mut empty = store.begin(1);

    empty.elements.clear();
    store.publish(window, empty);
    store.pointer(window, 15., 15., false);
    assert!(store.pick().unwrap().hovered.is_none());
}

#[test]
fn leases_stale_generations_and_click_sequences_are_bounded() {
    let store = Store::default();
    let window = WindowId(1);
    let mut snapshot = store.begin(1);

    let bounds = Layout {
        x: 0.,
        y: 0.,
        width: 100.,
        height: 100.,
    };

    snapshot
        .elements
        .push(boundary("element-1-0", None, bounds, bounds));
    store.publish(window, snapshot);
    assert!(matches!(
        store.highlight(Some("element-1-0"), "old"),
        Err(InspectionError::StaleGeneration)
    ));
    store.highlight(Some("element-1-0"), "1").unwrap();
    store.inspect_mode(true, 1000).unwrap();
    store.pointer(window, 5., 5., true);
    let first = store.pick().unwrap();

    assert_eq!(first.picked.as_deref(), Some("element-1-0"));
    assert_eq!(first.pick_sequence, 1);
    assert!(!first.mode);
    assert_eq!(store.pick().unwrap().pick_sequence, 1);
    assert!(matches!(
        store.inspect_mode(true, 2001),
        Err(InspectionError::InvalidLease)
    ));
    store.inspect_mode(true, 1).unwrap();
    store
        .0
        .borrow_mut()
        .expire(Instant::now() + Duration::from_secs(3));
    assert!(!store.pick().unwrap().mode);
    assert!(!store.pointer(window, 5., 5., false));
    assert!(store.0.borrow().windows[&window].highlight.is_none());
    store.inspect_mode(true, 1000).unwrap();
    store.0.borrow_mut().cancel();
    assert!(!store.pick().unwrap().mode);
}

#[test]
fn unchanged_paints_keep_generation_and_windows_never_share_highlights() {
    let store = Store::default();
    let mut snapshot = store.begin(1);

    let bounds = Layout {
        x: 0.,
        y: 0.,
        width: 10.,
        height: 10.,
    };

    snapshot
        .elements
        .push(boundary("element-1-0", None, bounds, bounds));
    store.publish(WindowId(1), snapshot);
    store.highlight(Some("element-1-0"), "1").unwrap();
    let mut same = store.begin(2);

    same.elements
        .push(boundary("element-2-0", None, bounds, bounds));
    store.publish(WindowId(1), same);
    assert_eq!(store.snapshot().capture, "1");
    assert!(store.0.borrow().windows[&WindowId(1)].highlight.is_some());
    let other = store.begin(2);

    store.publish(WindowId(2), other);
    assert!(store.0.borrow().windows[&WindowId(2)].highlight.is_none());
    store.inspect_mode(true, 1000).unwrap();
    store.pointer(WindowId(1), 1., 1., false);
    store.pointer(WindowId(2), 1., 1., false);
    let generation = store.pick().unwrap().generation;
    let first = store.0.borrow().windows[&WindowId(1)].snapshot.clone();

    store.publish(WindowId(1), first);
    assert_eq!(store.pick().unwrap().generation, generation);
    assert!(store.pick().unwrap().hovered.is_none());
    assert!(store.0.borrow().windows[&WindowId(1)].highlight.is_none());
}

#[test]
#[cfg(feature = "gpui")]
fn resolved_metadata_is_bounded_without_losing_boundaries() {
    let store = Store::default();
    let mut snapshot = store.begin(0);

    let bounds = Layout {
        x: 0.,
        y: 0.,
        width: 10.,
        height: 10.,
    };

    for index in 0..MAX_ELEMENTS {
        let mut element = boundary(&format!("element-1-{index}"), None, bounds, bounds);

        element.resolved = Some(ResolvedStyle {
            properties: serde_json::json!({"large":"x".repeat(8000)}),
            hovered: false,
        });
        snapshot.elements.push(element);
    }

    snapshot.bound_resolved_styles();
    assert_eq!(snapshot.elements.len(), MAX_ELEMENTS);
    assert_eq!(snapshot.omitted, 0);
    assert!(
        snapshot
            .elements
            .iter()
            .any(|element| element.resolved.is_some())
    );
    assert!(
        snapshot
            .elements
            .iter()
            .any(|element| element.resolved.is_none() && element.metadata_truncated)
    );
    assert!(serde_json::to_vec(&snapshot).unwrap().len() < MAX_SNAPSHOT_BYTES);
}
