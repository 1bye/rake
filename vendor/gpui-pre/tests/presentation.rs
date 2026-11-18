//! Verifies that presentation layers animate painted content without rendering views.

use gpui::{
    AtlasTextureId, AtlasTextureKind, AtlasTile, Bounds, ContentMask, Context, DevicePixels,
    ElementId, IntoElement, MonochromeSprite, PresentationTarget, PresentationTransform, Quad,
    Render, RequestFrameOptions, ScaledPixels, Scene, SpringConfig, Styled, TestAppContext, TileId,
    TransformationMatrix, VisualTestContext, Window, black, div, point, presentation_layer, px,
    size,
};
use std::{cell::Cell, rc::Rc};

const SPRING: SpringConfig = SpringConfig::new(300.0, 30.0, 1.0);

fn scaled_bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<ScaledPixels> {
    Bounds {
        origin: point(ScaledPixels(x), ScaledPixels(y)),
        size: size(ScaledPixels(width), ScaledPixels(height)),
    }
}

fn quad(bounds: Bounds<ScaledPixels>, mask: Bounds<ScaledPixels>) -> Quad {
    Quad {
        bounds,
        content_mask: ContentMask { bounds: mask },
        background: black().into(),
        ..Quad::default()
    }
}

fn transform(x: f32, y: f32, opacity: f32) -> PresentationTransform {
    PresentationTransform {
        offset: point(ScaledPixels(x), ScaledPixels(y)),
        opacity,
    }
}

fn moved(x: f32, opacity: f32) -> PresentationTarget {
    PresentationTarget {
        offset: point(px(x), px(0.0)),
        opacity,
    }
}

#[test]
fn compose_moves_and_fades_only_layer_primitives() {
    let window = scaled_bounds(0.0, 0.0, 400.0, 400.0);
    let mut source = Scene::default();

    source.insert_primitive(quad(scaled_bounds(0.0, 0.0, 10.0, 10.0), window));
    source.push_presentation_layer("card".into(), ContentMask { bounds: window });
    source.insert_primitive(quad(scaled_bounds(20.0, 20.0, 10.0, 10.0), window));
    source.pop_presentation_layer();

    let mut composed = Scene::default();
    composed.compose_presentation(&source, &|_| transform(5.0, 7.0, 0.5));

    assert_eq!(
        composed.quads[0].bounds.origin,
        point(ScaledPixels(0.0), ScaledPixels(0.0))
    );
    assert_eq!(
        composed.quads[1].bounds.origin,
        point(ScaledPixels(25.0), ScaledPixels(27.0))
    );
    assert_eq!(
        composed.quads[1].background.as_solid().map(|color| color.a),
        Some(0.5)
    );
}

#[test]
fn compose_keeps_layer_content_inside_its_outer_clip() {
    let outer = scaled_bounds(0.0, 0.0, 100.0, 100.0);
    let mut source = Scene::default();

    source.push_presentation_layer("card".into(), ContentMask { bounds: outer });
    source.insert_primitive(quad(scaled_bounds(80.0, 0.0, 10.0, 10.0), outer));
    source.pop_presentation_layer();

    let mut composed = Scene::default();
    composed.compose_presentation(&source, &|_| transform(15.0, 0.0, 1.0));

    let mask = composed.quads[0].content_mask.bounds;
    assert_eq!(mask.origin.x, ScaledPixels(15.0));
    assert_eq!(mask.size.width, ScaledPixels(85.0));
}

#[test]
fn compose_accumulates_nested_layers() {
    let window = scaled_bounds(0.0, 0.0, 400.0, 400.0);
    let mut source = Scene::default();

    source.push_presentation_layer("outer".into(), ContentMask { bounds: window });
    source.push_presentation_layer("inner".into(), ContentMask { bounds: window });
    source.insert_primitive(quad(scaled_bounds(10.0, 10.0, 10.0, 10.0), window));
    source.pop_presentation_layer();
    source.pop_presentation_layer();

    let mut composed = Scene::default();
    composed.compose_presentation(&source, &|id| {
        if *id == ElementId::from("outer") {
            transform(10.0, 0.0, 0.5)
        } else {
            transform(0.0, 20.0, 0.5)
        }
    });

    assert_eq!(
        composed.quads[0].bounds.origin,
        point(ScaledPixels(20.0), ScaledPixels(30.0))
    );
    assert_eq!(
        composed.quads[0].background.as_solid().map(|color| color.a),
        Some(0.25)
    );
}

#[test]
fn rotated_sprite_pivot_moves_with_the_layer() {
    let window = scaled_bounds(0.0, 0.0, 400.0, 400.0);
    let mut source = Scene::default();

    source.push_presentation_layer("icon".into(), ContentMask { bounds: window });
    source.insert_primitive(MonochromeSprite {
        order: 0,
        pad: 0,
        tile: AtlasTile {
            texture_id: AtlasTextureId {
                index: 0,
                kind: AtlasTextureKind::Monochrome,
            },
            tile_id: TileId(0),
            padding: 0,
            bounds: Bounds {
                origin: point(DevicePixels(0), DevicePixels(0)),
                size: size(DevicePixels(8), DevicePixels(8)),
            },
        },
        bounds: scaled_bounds(10.0, 10.0, 8.0, 8.0),
        content_mask: ContentMask { bounds: window },
        color: black(),
        transformation: TransformationMatrix {
            rotation_scale: [[0.0, -1.0], [1.0, 0.0]],
            translation: [0.0, 0.0],
        },
    });
    source.pop_presentation_layer();

    let mut composed = Scene::default();
    composed.compose_presentation(&source, &|_| transform(10.0, 0.0, 1.0));

    assert_eq!(
        composed.monochrome_sprites[0].transformation.translation,
        [10.0, -10.0]
    );
}

struct CountingView {
    renders: Rc<Cell<usize>>,
}

impl Render for CountingView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);

        presentation_layer("card", div().size(px(40.0)).bg(black()))
    }
}

fn counting_window(cx: &mut TestAppContext) -> (Rc<Cell<usize>>, &mut VisualTestContext) {
    let renders = Rc::new(Cell::new(0));
    let view_renders = renders.clone();
    let (_, cx) = cx.add_window_view(|_, _| CountingView {
        renders: view_renders,
    });

    cx.run_until_parked();
    (renders, cx)
}

#[gpui::test]
fn motion_frames_present_without_rendering_views(cx: &mut TestAppContext) {
    let (renders, cx) = counting_window(cx);
    let renders_before = renders.get();

    cx.update(|window, cx| {
        window.animate_presentation_layer("card", moved(120.0, 0.5), SPRING, cx);
    });

    for _ in 0..4 {
        cx.simulate_frame_request(RequestFrameOptions::default());
    }

    assert_eq!(renders.get(), renders_before);
    assert!(cx.update(|window, _| window.is_presentation_layer_animating("card")));
}

#[gpui::test]
fn presenting_uses_the_composed_scene(cx: &mut TestAppContext) {
    let (renders, cx) = counting_window(cx);
    let renders_before = renders.get();

    cx.update(|window, _| window.set_presentation_layer("card", moved(12.0, 0.5)));
    cx.simulate_frame_request(RequestFrameOptions::default());

    cx.update(|window, _| {
        let scale_factor = window.scale_factor();
        let rendered = window.rendered_scene_for_test().quads[0];
        let composed = window.presented_scene_for_test().quads[0];

        assert_eq!(
            composed.bounds.origin.x,
            rendered.bounds.origin.x + ScaledPixels(12.0 * scale_factor)
        );
        assert_eq!(
            composed.background.as_solid().map(|color| color.a),
            Some(0.5)
        );
    });
    assert_eq!(renders.get(), renders_before);
}

#[gpui::test]
fn retargeting_preserves_momentum(cx: &mut TestAppContext) {
    let (_, cx) = counting_window(cx);

    cx.update(|window, cx| {
        window.animate_presentation_layer("card", moved(100.0, 1.0), SPRING, cx);
        window.advance_presentation_layers_for_test(0.05);

        let before = window.presentation_layer_value("card").offset.x;

        window.animate_presentation_layer("card", PresentationTarget::default(), SPRING, cx);
        window.advance_presentation_layers_for_test(0.001);

        assert!(window.presentation_layer_value("card").offset.x > before);
    });
}

#[gpui::test]
fn settled_layers_return_to_the_painted_scene(cx: &mut TestAppContext) {
    let (_, cx) = counting_window(cx);

    cx.update(|window, cx| {
        window.set_presentation_layer("card", moved(40.0, 1.0));
        window.animate_presentation_layer("card", PresentationTarget::default(), SPRING, cx);
        window.advance_presentation_layers_for_test(5.0);

        assert!(!window.is_presentation_layer_animating("card"));
        assert_eq!(
            window.presentation_layer_value("card"),
            PresentationTarget::default()
        );
    });
}

#[gpui::test]
fn invalid_targets_and_springs_do_not_animate(cx: &mut TestAppContext) {
    let (_, cx) = counting_window(cx);
    let invalid_target = PresentationTarget {
        offset: point(px(f32::NAN), px(4.0)),
        opacity: 8.0,
    };

    cx.update(|window, cx| {
        window.animate_presentation_layer(
            "card",
            invalid_target,
            SpringConfig::new(0.0, 1.0, 1.0),
            cx,
        );

        assert!(!window.is_presentation_layer_animating("card"));
        assert_eq!(
            window.presentation_layer_value("card"),
            PresentationTarget::default()
        );
    });
}

#[gpui::test]
fn reduced_motion_jumps_to_the_target(cx: &mut TestAppContext) {
    let (_, cx) = counting_window(cx);
    let target = moved(30.0, 1.0);

    cx.update(|window, cx| {
        cx.set_reduce_motion(true);
        window.animate_presentation_layer("card", target, SPRING, cx);

        assert!(!window.is_presentation_layer_animating("card"));
        assert_eq!(window.presentation_layer_value("card"), target);
    });
}
