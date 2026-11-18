//! Spring-driven layers that move and fade painted content without rendering views.
//!
//! A view states where content belongs through normal layout. A presentation layer then animates
//! a visual offset and opacity toward a target on frames that skip render, layout, prepaint and
//! paint. Hit testing stays at the laid-out position, and fading applies to each primitive rather
//! than to a flattened group.

use crate::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Point, PresentationTransform, ScaledPixels, Scene, SpringConfig, SpringState,
    Window, point, px,
};
use collections::FxHashMap;
use scheduler::Instant;

const OFFSET_EPSILON_PX: f32 = 0.01;
const OPACITY_EPSILON: f32 = 0.001;

/// The visual state a presentation layer shows relative to its laid-out position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PresentationTarget {
    /// The visual offset from the layer's laid-out position.
    pub offset: Point<Pixels>,
    /// The layer's opacity from `0.0` to `1.0`.
    pub opacity: f32,
}

impl Default for PresentationTarget {
    fn default() -> Self {
        Self {
            offset: point(px(0.0), px(0.0)),
            opacity: 1.0,
        }
    }
}

impl PresentationTarget {
    fn sanitized(self) -> Self {
        let finite_offset =
            f32::from(self.offset.x).is_finite() && f32::from(self.offset.y).is_finite();

        Self {
            offset: if finite_offset {
                self.offset
            } else {
                Self::default().offset
            },
            opacity: if self.opacity.is_finite() {
                self.opacity.clamp(0.0, 1.0)
            } else {
                1.0
            },
        }
    }

    fn is_identity(self) -> bool {
        self == Self::default()
    }
}

#[derive(Clone, Copy, Debug)]
struct PresentationTrack {
    config: SpringConfig,
    target: PresentationTarget,
    x: SpringState,
    y: SpringState,
    opacity: SpringState,
    settled: bool,
}

impl PresentationTrack {
    fn at_rest(target: PresentationTarget, config: SpringConfig) -> Self {
        Self {
            config,
            target,
            x: SpringState {
                position: target.offset.x.into(),
                velocity: 0.0,
            },
            y: SpringState {
                position: target.offset.y.into(),
                velocity: 0.0,
            },
            opacity: SpringState {
                position: target.opacity,
                velocity: 0.0,
            },
            settled: true,
        }
    }

    fn value(&self) -> PresentationTarget {
        PresentationTarget {
            offset: point(px(self.x.position), px(self.y.position)),
            opacity: self.opacity.position.clamp(0.0, 1.0),
        }
    }

    fn advance(&mut self, delta_seconds: f32) {
        let target_x = f32::from(self.target.offset.x);
        let target_y = f32::from(self.target.offset.y);

        self.x = self.config.step(self.x, target_x, delta_seconds);
        self.y = self.config.step(self.y, target_y, delta_seconds);
        self.opacity = self
            .config
            .step(self.opacity, self.target.opacity, delta_seconds);
        self.settled = self.config.is_settled(self.x, target_x, OFFSET_EPSILON_PX)
            && self.config.is_settled(self.y, target_y, OFFSET_EPSILON_PX)
            && self
                .config
                .is_settled(self.opacity, self.target.opacity, OPACITY_EPSILON);

        if self.settled {
            *self = Self::at_rest(self.target, self.config);
        }
    }
}

fn is_valid_spring(config: SpringConfig) -> bool {
    config.stiffness.is_finite()
        && config.stiffness > 0.0
        && config.mass.is_finite()
        && config.mass > 0.0
        && config.damping.is_finite()
        && config.damping > 0.0
}

/// Window-owned presentation tracks and the scene composed from them.
#[derive(Default)]
pub(crate) struct PresentationLayers {
    tracks: FxHashMap<ElementId, PresentationTrack>,
    last_advance: Option<Instant>,
    pub(crate) composed: Scene,
}

impl PresentationLayers {
    fn set(&mut self, id: ElementId, target: PresentationTarget) {
        let target = target.sanitized();

        if target.is_identity() {
            self.tracks.remove(&id);
        } else {
            self.tracks.insert(
                id,
                PresentationTrack::at_rest(target, SpringConfig::new(1.0, 2.0, 1.0)),
            );
        }
    }

    fn animate(&mut self, id: ElementId, target: PresentationTarget, config: SpringConfig) {
        let target = target.sanitized();

        if !is_valid_spring(config) {
            self.set(id, target);
            return;
        }

        let track = self
            .tracks
            .entry(id)
            .or_insert_with(|| PresentationTrack::at_rest(PresentationTarget::default(), config));

        track.config = config;
        track.target = target;
        track.settled = false;
    }

    fn advance(&mut self, delta_seconds: f32) {
        for track in self.tracks.values_mut().filter(|track| !track.settled) {
            track.advance(delta_seconds);
        }

        self.tracks
            .retain(|_, track| !track.settled || !track.target.is_identity());
    }

    fn is_animating(&self) -> bool {
        self.tracks.values().any(|track| !track.settled)
    }

    pub(crate) fn has_transforms(&self) -> bool {
        !self.tracks.is_empty()
    }

    fn transform(&self, id: &ElementId, scale_factor: f32) -> PresentationTransform {
        let value = self
            .tracks
            .get(id)
            .map_or_else(PresentationTarget::default, PresentationTrack::value);

        // Whole device pixels keep glyphs and their boxes moving together without resampling.
        PresentationTransform {
            offset: point(
                ScaledPixels((f32::from(value.offset.x) * scale_factor).round()),
                ScaledPixels((f32::from(value.offset.y) * scale_factor).round()),
            ),
            opacity: value.opacity,
        }
    }

    pub(crate) fn compose(&mut self, source: &Scene, scale_factor: f32) -> &Scene {
        let mut composed = std::mem::take(&mut self.composed);

        composed.compose_presentation(source, &|id| self.transform(id, scale_factor));
        self.composed = composed;
        &self.composed
    }
}

impl Window {
    /// Springs a presentation layer toward `target` without rendering any view.
    ///
    /// Velocity carries across retargeting. When [`App::reduce_motion`] is set, the layer moves
    /// to `target` immediately. An invalid spring also moves the layer immediately.
    pub fn animate_presentation_layer(
        &mut self,
        id: impl Into<ElementId>,
        target: PresentationTarget,
        config: SpringConfig,
        cx: &App,
    ) {
        if cx.reduce_motion() {
            self.set_presentation_layer(id, target);
            return;
        }

        if !self.presentation_layers.is_animating() {
            self.presentation_layers.last_advance = Some(Instant::now());
        }

        self.presentation_layers.animate(id.into(), target, config);
        self.sync_presentation_frames();
    }

    /// Moves a presentation layer to `target` immediately, cancelling any running spring.
    pub fn set_presentation_layer(&mut self, id: impl Into<ElementId>, target: PresentationTarget) {
        self.presentation_layers.set(id.into(), target);
        self.needs_present.set(true);
        self.sync_presentation_frames();
    }

    /// Returns the value a presentation layer currently shows.
    pub fn presentation_layer_value(&self, id: impl Into<ElementId>) -> PresentationTarget {
        self.presentation_layers
            .tracks
            .get(&id.into())
            .map_or_else(PresentationTarget::default, PresentationTrack::value)
    }

    /// Returns whether a presentation layer's spring is still moving.
    pub fn is_presentation_layer_animating(&self, id: impl Into<ElementId>) -> bool {
        self.presentation_layers
            .tracks
            .get(&id.into())
            .is_some_and(|track| !track.settled)
    }

    pub(crate) fn advance_presentation_layers(&mut self, now: Instant) {
        let last_advance = self.presentation_layers.last_advance.unwrap_or(now);
        let delta_seconds = now.saturating_duration_since(last_advance).as_secs_f32();

        self.presentation_layers.last_advance = Some(now);
        self.presentation_layers.advance(delta_seconds);
        self.presentation_active
            .set(self.presentation_layers.is_animating());
    }

    /// Advances presentation springs by `delta_seconds` without waiting for a platform frame.
    #[cfg(any(test, feature = "test-support"))]
    pub fn advance_presentation_layers_for_test(&mut self, delta_seconds: f32) {
        self.presentation_layers.advance(delta_seconds);
        self.presentation_active
            .set(self.presentation_layers.is_animating());
    }

    /// Returns the scene most recently composed for presentation.
    #[cfg(any(test, feature = "test-support"))]
    pub fn presented_scene_for_test(&self) -> &Scene {
        &self.presentation_layers.composed
    }

    /// Returns the scene most recently painted by views.
    #[cfg(any(test, feature = "test-support"))]
    pub fn rendered_scene_for_test(&self) -> &Scene {
        &self.rendered_frame.scene
    }

    fn sync_presentation_frames(&mut self) {
        self.presentation_active
            .set(self.presentation_layers.is_animating());
        self.needs_present.set(true);
        self.platform_window.schedule_frame();
        self.invalidator.wake_platform();
    }
}

/// Wraps `child` in a layer that [`Window::animate_presentation_layer`] can move and fade.
pub fn presentation_layer(id: impl Into<ElementId>, child: impl IntoElement) -> PresentationLayer {
    PresentationLayer {
        id: id.into(),
        child: Some(child.into_any_element()),
    }
}

/// An element whose painted content can animate without rendering its view.
pub struct PresentationLayer {
    id: ElementId,
    child: Option<AnyElement>,
}

impl IntoElement for PresentationLayer {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for PresentationLayer {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut child = self
            .child
            .take()
            .expect("a presentation layer requests layout once per frame");

        (child.request_layout(window, cx), child)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let content_mask = window.content_mask().scale(window.scale_factor());

        window
            .next_frame
            .scene
            .push_presentation_layer(self.id.clone(), content_mask);
        child.paint(window, cx);
        window.next_frame.scene.pop_presentation_layer();
    }
}
