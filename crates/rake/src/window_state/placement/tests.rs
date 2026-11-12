//! Covers: display changes, logical coordinates, and corrupt preference records.
//! Real: placement and versioned JSON serialization. Doubles: display snapshots.
//! Requires: none.

use super::*;

fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn display(uuid: &str, visible: Rect) -> Display {
    Display {
        uuid: Some(uuid.into()),
        visible,
    }
}

fn saved(bounds: Rect) -> SavedWindow {
    SavedWindow {
        bounds,
        display: Some("external".into()),
        mode: Mode::Windowed,
    }
}

#[test]
fn partial_offscreen_bounds_clamp_including_negative_coordinates() {
    let displays = [display("external", rect(-1920., 30., 1920., 1050.))];

    assert_eq!(
        restore(
            &saved(rect(-2100., -50., 900., 600.)),
            &displays,
            0,
            rect(0., 0., 1., 1.)
        ),
        Some((0, rect(-1920., 30., 900., 600.)))
    );
}

#[test]
fn disconnected_display_uses_defaults_even_when_coordinates_overlap() {
    let displays = [display("internal", rect(0., 25., 1440., 875.))];
    let default = rect(100., 100., 800., 600.);

    assert_eq!(
        restore(&saved(rect(200., 200., 700., 500.)), &displays, 0, default),
        Some((0, default))
    );
}

#[test]
fn fully_offscreen_bounds_use_defaults() {
    let displays = [display("external", rect(0., 25., 1440., 875.))];
    let default = rect(100., 100., 800., 600.);

    assert_eq!(
        restore(
            &saved(rect(4000., 3000., 700., 500.)),
            &displays,
            0,
            default
        ),
        Some((0, default))
    );
}

#[test]
fn oversized_window_and_offscreen_defaults_recenter_and_fit() {
    let displays = [display("internal", rect(0., 25., 1000., 700.))];

    assert_eq!(
        restore(
            &saved(rect(4000., 0., 2000., 900.)),
            &displays,
            0,
            rect(4000., 0., 2000., 900.)
        ),
        Some((0, rect(0., 25., 1000., 700.)))
    );
    assert_eq!(
        restore(
            &saved(rect(4000., 0., 600., 400.)),
            &displays,
            0,
            rect(4000., 0., 600., 400.)
        ),
        Some((0, rect(200., 175., 600., 400.)))
    );
}

#[test]
fn mixed_scales_preserve_logical_size_and_identity_after_display_reordering() {
    // A 3840x2160 display at 2x exposes the same logical size as 1920x1080 at 1x.
    let bounds = rect(2100., 100., 800., 600.);
    let state = saved(bounds);
    let displays = [
        display("internal", rect(0., 0., 1920., 1080.)),
        display("external", rect(1920., 0., 1920., 1080.)),
    ];

    assert_eq!(
        restore(&state, &displays, 0, rect(0., 0., 100., 100.)),
        Some((1, bounds))
    );
    let reordered = [
        display("external", displays[1].visible),
        display("internal", displays[0].visible),
    ];

    assert_eq!(
        restore(&state, &reordered, 1, rect(0., 0., 100., 100.)),
        Some((0, bounds))
    );
}

#[test]
fn scale_change_shrinking_logical_work_area_clamps_without_rescaling() {
    let displays = [display("external", rect(0., 20., 1280., 700.))];

    assert_eq!(
        restore(
            &saved(rect(1000., 500., 800., 600.)),
            &displays,
            0,
            rect(0., 0., 1., 1.)
        ),
        Some((0, rect(480., 120., 800., 600.)))
    );
}

#[test]
fn absent_uuid_matches_visible_bounds_and_no_displays_leave_defaults() {
    let mut state = saved(rect(100., 100., 500., 400.));
    state.display = None;
    let visible = rect(0., 25., 1440., 875.);

    assert_eq!(
        restore(&state, &[display("internal", visible)], 0, visible),
        Some((0, state.bounds))
    );
    assert_eq!(restore(&state, &[], 0, visible), None);
    assert_eq!(
        restore(
            &state,
            &[display("internal", rect(0., 0., 0., 0.))],
            0,
            visible
        ),
        None
    );
}

#[test]
fn serialization_preserves_independent_windows_and_all_modes() {
    let records = [Mode::Windowed, Mode::Maximized, Mode::Fullscreen]
        .into_iter()
        .enumerate()
        .map(|(index, mode)| {
            let mut state = saved(rect(index as f32 * 50., 50., 800., 600.));
            state.mode = mode;

            (format!("document-{index}"), state)
        })
        .collect();

    assert_eq!(decode(&encode(&records)), records);
}

#[test]
fn corrupt_records_preserve_other_windows_and_unknown_versions_use_defaults() {
    let good = saved(rect(100., 100., 800., 600.));
    let value = serde_json::json!({"version": 1, "windows": {
        "main": good, "broken": {"mode": "windowed"},
        "zero": {"bounds": {"x": 0, "y": 0, "width": 0, "height": 5}, "display": null, "mode": "windowed"}
    }});

    assert_eq!(
        decode(&value.to_string()),
        BTreeMap::from([("main".into(), good)])
    );
    for value in ["", "{", "null", r#"{"version":2,"windows":{}}"#] {
        assert!(decode(value).is_empty(), "invalid document: {value}");
    }
}

#[test]
fn nonfinite_and_unreasonable_bounds_are_rejected() {
    let displays = [display("external", rect(0., 0., 1000., 800.))];

    for bounds in [
        rect(f32::NAN, 0., 10., 10.),
        rect(0., 0., f32::INFINITY, 10.),
        rect(0., 0., -1., 10.),
        rect(2_000_000., 0., 10., 10.),
    ] {
        assert_eq!(
            restore(&saved(bounds), &displays, 0, displays[0].visible),
            None
        );
    }
}
