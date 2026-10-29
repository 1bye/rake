//! Covers: runtime identity, verified reload lineage, stale captures and selection races.
//! Real: Inspector state and shared wire types. Doubles: supplied owned snapshots.
//! Requires: no network, display or GPU.

use rake::control::RuntimeInfo;
use rake::inspection::{Element, Snapshot, Status};
use rake::router::RuntimeId;
use rake_inspector::{Inspector, Instance, Update};

fn runtime(id: u8) -> RuntimeId {
    serde_json::from_value(serde_json::json!(format!(
        "00000000-0000-4000-8000-{id:012}"
    )))
    .unwrap()
}

fn instance(id: u8, predecessor: Option<u8>) -> Instance {
    Instance {
        application_id: "test.app".into(),
        info: RuntimeInfo {
            element_inspection: Status::Captured,
            element_interaction: true,
            protocol: 1,
            runtime_id: runtime(id),
            platform: "native".into(),
            commands: vec![],
            queries: vec!["inspect_tree".into(), "inspect_element".into()],
            supported_targets: vec![],
            completed_operations: 16,
            pending_operations: 16,
            idempotency_capacity: 16,
        },
        restored_from: predecessor.map(runtime),
    }
}

fn element(id: &str) -> Element {
    Element {
        id: id.into(),
        parent: None,
        kind: "gpui::Div".into(),
        semantic_id: Some("document.body".into()),
        source: None,
        layout: None,
        content_mask: None,
        styles: None,
        resolved: None,
        stable_id: None,
        metadata_truncated: false,
        tab_id: None,
        entry_id: None,
    }
}

fn tree(capture: &str, id: &str) -> Snapshot {
    Snapshot {
        status: Status::Captured,
        capture: capture.into(),
        router_revision: 1,
        elements: vec![element(id)],
        omitted: 0,
        coverage: "boundaries".into(),
    }
}

#[test]
fn replaced_captures_keep_frozen_details_and_reject_late_selection_responses() {
    let mut inspector = Inspector::new(None);

    inspector.apply(Update::Instances(0, Ok(vec![instance(1, None)])));
    let epoch = inspector.epoch;

    inspector.apply(Update::Tree(
        epoch,
        runtime(1),
        Ok(tree("1", "element-1-0")),
    ));
    inspector.select_element("element-1-0");
    inspector.apply(Update::Element(
        epoch,
        runtime(1),
        "element-1-0".into(),
        Box::new(Ok(element("element-1-0"))),
    ));
    inspector.apply(Update::Tree(
        epoch,
        runtime(1),
        Ok(tree("2", "element-2-0")),
    ));
    assert!(inspector.stale());
    assert_eq!(inspector.detail.as_ref().unwrap().id, "element-1-0");
    inspector.select_element("element-2-0");
    inspector.apply(Update::Element(
        epoch,
        runtime(1),
        "element-1-0".into(),
        Box::new(Ok(element("element-1-0"))),
    ));
    assert_eq!(inspector.detail.as_ref().unwrap().id, "element-2-0");
    inspector.apply(Update::Element(
        epoch,
        runtime(1),
        "element-2-0".into(),
        Box::new(Err("NotRetained".into())),
    ));
    inspector.apply(Update::Tree(
        epoch,
        runtime(1),
        Ok(tree("3", "element-3-0")),
    ));
    assert_eq!(inspector.lookup_error.as_deref(), Some("NotRetained"));
}

#[test]
fn only_a_proven_successor_follows_a_restart_and_old_epoch_results_cannot_attach() {
    let mut inspector = Inspector::new(None);

    inspector.apply(Update::Instances(0, Ok(vec![instance(1, None)])));
    let epoch = inspector.epoch;

    inspector.apply(Update::Instances(epoch, Ok(vec![instance(2, None)])));
    assert_eq!(
        inspector.runtime,
        Some(runtime(1)),
        "matching application names do not prove lineage"
    );
    inspector.apply(Update::Instances(epoch, Ok(vec![instance(2, Some(1))])));
    assert_eq!(inspector.runtime, Some(runtime(2)));
    assert!(inspector.tree.is_none());
    inspector.apply(Update::Tree(
        epoch,
        runtime(1),
        Ok(tree("old", "old-element")),
    ));
    assert!(inspector.tree.is_none());
}

#[test]
fn ambiguous_runtime_selection_is_explicit_and_filtering_keeps_exact_ids() {
    let mut inspector = Inspector::new(None);

    inspector.apply(Update::Instances(
        0,
        Ok(vec![instance(1, None), instance(2, None)]),
    ));
    assert!(inspector.runtime.is_none());
    inspector.select_runtime(Some(runtime(2)));
    inspector.apply(Update::Tree(
        inspector.epoch,
        runtime(2),
        Ok(tree("2", "element-2-0")),
    ));
    inspector.filter = "DOCUMENT.BODY".into();
    assert_eq!(inspector.visible_elements()[0].id, "element-2-0");
    inspector.select_element("not-present");
    assert!(inspector.selected.is_none());
    inspector.disconnected = true;
    inspector.select_runtime(inspector.runtime);
    inspector.apply(Update::Tree(
        inspector.epoch,
        runtime(2),
        Ok(tree("3", "element-3-0")),
    ));
    assert!(inspector.tree.is_none());
}

#[test]
fn picker_waits_for_its_exact_capture_and_never_replays_a_click() {
    let mut model = Inspector::default();

    model.apply(Update::Instances(0, Ok(vec![instance(1, None)])));
    let epoch = model.epoch;

    model.apply(Update::Tree(
        epoch,
        runtime(1),
        Ok(tree("1", "element-1-0")),
    ));
    let pick = rake::inspection::Pick {
        mode: false,
        generation: "2".into(),
        hovered: Some("element-2-0".into()),
        picked: Some("element-2-0".into()),
        pick_sequence: 1,
    };

    model.apply(Update::Pick(epoch, runtime(1), Ok(pick.clone())));
    assert!(model.selected.is_none());
    model.apply(Update::Tree(
        epoch,
        runtime(1),
        Ok(tree("2", "element-2-0")),
    ));
    assert_eq!(model.selected.as_deref(), Some("element-2-0"));
    model
        .tree
        .as_mut()
        .unwrap()
        .elements
        .push(element("manual-selection"));
    model.select_element("manual-selection");
    model.apply(Update::Pick(epoch, runtime(1), Ok(pick)));
    assert_eq!(model.selected.as_deref(), Some("manual-selection"));
}

#[test]
fn live_resolved_styles_update_the_selection_without_replacing_its_capture() {
    let mut inspector = Inspector::new(None);

    inspector.apply(Update::Instances(0, Ok(vec![instance(1, None)])));
    let epoch = inspector.epoch;

    inspector.apply(Update::Tree(
        epoch,
        runtime(1),
        Ok(tree("live", "element-live-0")),
    ));
    inspector.select_element("element-live-0");
    let mut changed = tree("live", "element-live-0");

    changed.elements[0].resolved = Some(rake::inspection::ResolvedStyle {
        properties: serde_json::json!({"background":"hover color"}),
        hovered: true,
    });
    inspector.apply(Update::Tree(epoch, runtime(1), Ok(changed)));
    assert!(!inspector.stale());
    assert_eq!(inspector.selected.as_deref(), Some("element-live-0"));
    assert!(
        inspector
            .detail
            .as_ref()
            .unwrap()
            .resolved
            .as_ref()
            .unwrap()
            .hovered
    );
    assert_eq!(
        inspector
            .detail
            .as_ref()
            .unwrap()
            .resolved
            .as_ref()
            .unwrap()
            .properties["background"],
        "hover color"
    );
}
