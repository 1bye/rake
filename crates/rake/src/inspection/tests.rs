//! Covers: exact capture lookup, unavailable/waiting states and snapshot replacement.
//! Real: owned store and serialization. Doubles: captured metadata, no GPUI traversal.
//! Requires: none.

use super::*;
use serde_json::Value;

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
    Rc::make_mut(&mut store.0.borrow_mut().latest).status = Status::Waiting;
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
        view: None,
        layout: None,
        content_mask: None,
        box_model: None,
        styles: None,
        resolved: None,
        stable_id: None,
        navigation: Default::default(),
        metadata_truncated: false,
        paint_order: None,
        style_editable: false,
        style_overrides: Vec::new(),
        accessibility: None,
        handlers: Vec::new(),
        tab_id: None,
        entry_id: None,
    };

    let mut state = store.0.borrow_mut();
    let snapshot = Rc::make_mut(&mut state.latest);

    snapshot.status = Status::Captured;
    snapshot.elements.push(element);
    drop(state);
    let owned = store.element("element-1-0").unwrap();

    Rc::make_mut(&mut store.0.borrow_mut().latest).elements[0].id = "element-2-0".into();
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
        view: None,
        layout: None,
        content_mask: None,
        box_model: None,
        styles: None,
        resolved: None,
        stable_id: None,
        navigation: Default::default(),
        metadata_truncated: false,
        paint_order: None,
        style_editable: false,
        style_overrides: Vec::new(),
        accessibility: None,
        handlers: Vec::new(),
        tab_id: None,
        entry_id: None,
    };

    let mut snapshot = store.snapshot();
    let mut bytes = 0;

    for index in 0..MAX_ELEMENTS + 10 {
        let mut element = template.clone();

        element.id = format!("element-1-{index}");
        retain(&mut snapshot, element, &mut bytes);
    }

    assert_eq!(snapshot.elements.len(), MAX_ELEMENTS);
    assert_eq!(snapshot.omitted, 10);
    assert!(serde_json::to_vec(&snapshot).unwrap().len() < MAX_CAPTURE_BYTES);
    let mut snapshot = store.snapshot();
    let mut bytes = 0;
    let mut element = template;

    element.styles = Some(Value::String("\\".repeat(4000)));

    for _ in 0..MAX_ELEMENTS {
        retain(&mut snapshot, element.clone(), &mut bytes);
    }

    assert!(snapshot.elements.len() < MAX_ELEMENTS);
    assert!(snapshot.omitted > 0);
    assert!(serde_json::to_vec(&snapshot).unwrap().len() < MAX_CAPTURE_BYTES);
}

fn boundary(id: &str, parent: Option<&str>, bounds: Layout, mask: Layout) -> Element {
    Element {
        id: id.into(),
        parent: parent.map(str::to_owned),
        kind: "Div".into(),
        semantic_id: None,
        source: None,
        view: None,
        layout: Some(bounds),
        content_mask: Some(mask),
        box_model: None,
        styles: None,
        resolved: None,
        stable_id: None,
        navigation: Default::default(),
        metadata_truncated: false,
        paint_order: None,
        style_editable: false,
        style_overrides: Vec::new(),
        accessibility: None,
        handlers: Vec::new(),
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

    let first = store.0.borrow().windows[&WindowId(1)]
        .snapshot
        .as_ref()
        .clone();

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
            layers: Vec::new(),
        });
        snapshot.elements.push(element);
    }

    bound_resolved_styles(&mut snapshot);
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
    assert!(serde_json::to_vec(&snapshot).unwrap().len() < MAX_CAPTURE_BYTES);
}

/// Manual CPU/allocation benchmark; synthetic metadata, no GPU rendering.
#[test]
#[ignore = "manual benchmark: cargo test -p rake --no-default-features --release --lib publish_benchmark -- --ignored --nocapture"]
fn publish_benchmark() {
    use std::hint::black_box;
    use std::time::Instant;

    let mut samples = Vec::new();

    for _ in 0..5 {
        let store = Store::default();
        let mut elapsed = std::time::Duration::ZERO;

        for _ in 0..200 {
            let mut snapshot = store.begin(1);

            for index in 0..256 {
                snapshot.elements.push(Element {
                    id: format!("element-{}-{index}", snapshot.capture),
                    parent: (index > 0).then(|| format!("element-{}-0", snapshot.capture)),
                    kind: "gpui::Div".into(),
                    semantic_id: Some(format!("row-{index}")),
                    source: None,
                    view: None,
                    layout: Some(Layout {
                        x: 0.,
                        y: index as f32,
                        width: 400.,
                        height: 1.,
                    }),
                    box_model: None,
                    content_mask: Some(Layout {
                        x: 0.,
                        y: 0.,
                        width: 400.,
                        height: 400.,
                    }),
                    styles: Some(serde_json::json!({"metadata": "x".repeat(512)})),
                    resolved: None,
                    stable_id: None,
                    navigation: Default::default(),
                    metadata_truncated: false,
                    paint_order: None,
                    style_editable: false,
                    style_overrides: Vec::new(),
                    accessibility: None,
                    handlers: Vec::new(),
                    tab_id: None,
                    entry_id: None,
                });
            }

            let started = Instant::now();

            store.publish(WindowId(1), black_box(snapshot));
            elapsed += started.elapsed();
        }

        samples.push(elapsed.as_secs_f64() * 1e6 / 200.);
    }

    samples.sort_by(f64::total_cmp);
    println!(
        "inspection publish, 256 stable boundaries: {:.1} us",
        samples[2]
    );
}

#[test]
fn stable_capture_refreshes_styles_and_owned_snapshots_stay_independent() {
    let store = Store::default();

    let bounds = Layout {
        x: 0.,
        y: 0.,
        width: 10.,
        height: 10.,
    };

    let mut first = store.begin(1);

    first
        .elements
        .push(boundary("element-1-0", None, bounds, bounds));
    first
        .elements
        .push(boundary("element-1-1", Some("element-1-0"), bounds, bounds));
    store.publish(WindowId(1), first);
    let owned = store.snapshot();
    let mut next = store.begin(2);

    next.elements
        .push(boundary("element-2-0", None, bounds, bounds));
    next.elements
        .push(boundary("element-2-1", Some("element-2-0"), bounds, bounds));
    next.elements[1].styles = Some(serde_json::json!({"opacity": 0.5}));
    store.publish(WindowId(1), next);
    let refreshed = store.snapshot();

    assert_eq!(refreshed.capture, "1");
    assert_eq!(refreshed.router_revision, 2);
    assert_eq!(refreshed.elements[1].id, "element-1-1");
    assert_eq!(refreshed.elements[1].parent.as_deref(), Some("element-1-0"));
    assert_eq!(
        refreshed.elements[1].styles,
        Some(serde_json::json!({"opacity": 0.5}))
    );
    assert!(owned.elements[1].styles.is_none());
    assert_eq!(owned.router_revision, 1);
    let mut changed = store.begin(3);

    changed.elements = refreshed.elements;

    for (index, element) in changed.elements.iter_mut().enumerate() {
        element.id = format!("element-3-{index}");
    }

    changed.elements[1].parent = None;
    store.publish(WindowId(1), changed);
    assert_eq!(store.snapshot().capture, "3");
    assert!(matches!(
        store.element("element-1-0"),
        Err(InspectionError::NotRetained)
    ));
}

#[test]
fn tree_pages_retain_large_indices_and_pin_capture_during_redraw() {
    let store = Store::default();
    let bounds = Layout {
        x: 0.,
        y: 0.,
        width: 10.,
        height: 10.,
    };
    let mut snapshot = store.begin(0);
    for i in 0..1000 {
        let mut element = boundary(
            &format!("element-{}-{i}", snapshot.capture),
            None,
            bounds,
            bounds,
        );
        element.styles = Some(serde_json::json!({"color":"red"}));
        snapshot.elements.push(element);
    }
    store.publish(WindowId(1), snapshot);
    let first = store.tree_page(None, 0).unwrap();
    assert_eq!(first.total, 1000);
    assert_eq!(first.next, Some(TREE_PAGE_SIZE));
    assert!(
        first
            .snapshot
            .elements
            .iter()
            .all(|element| element.styles.is_none())
    );
    assert_eq!(store.snapshot().elements.len(), TREE_PAGE_SIZE);
    assert_eq!(store.snapshot().omitted, 1000 - TREE_PAGE_SIZE);
    let next = store.begin(1);
    store.publish(WindowId(1), next);
    let second = store
        .tree_page(Some(&first.snapshot.capture), TREE_PAGE_SIZE)
        .unwrap();
    assert_eq!(second.snapshot.capture, first.snapshot.capture);
    assert!(
        store
            .element(&second.snapshot.elements[0].id)
            .unwrap()
            .styles
            .is_some()
    );
    assert!(matches!(
        store.tree_page(Some("expired"), 0),
        Err(InspectionError::StaleGeneration)
    ));
    assert!(matches!(
        store.tree_page(Some(&first.snapshot.capture), 1001),
        Err(InspectionError::InvalidPage)
    ));
    assert!(serde_json::to_vec(&first).unwrap().len() < MAX_SNAPSHOT_BYTES);
}

#[test]
fn picking_uses_paint_order_excludes_unpainted_and_respects_visible_mask() {
    let store = Store::default();
    let full = Layout {
        x: 10.,
        y: 20.,
        width: 100.,
        height: 100.,
    };
    let clip = Layout { width: 20., ..full };
    let mut snapshot = store.begin(0);
    let mut front = boundary("front", None, full, clip);
    front.paint_order = Some(3);
    let mut back = boundary("back", None, full, full);
    back.paint_order = Some(1);
    let hidden = boundary("not-painted", None, full, full);
    snapshot.elements = vec![front, back, hidden];
    assert_eq!(hit_test(&snapshot, 15., 25.).unwrap().id, "front");
    assert_eq!(hit_test(&snapshot, 50., 25.).unwrap().id, "back");
    assert!(hit_test(&snapshot, f32::NAN, 25.).is_none());
    assert!(hit_test(&snapshot, 10., 120.).is_none());
}
