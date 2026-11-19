//! Springs that move and fade painted content without rendering views.
//!
//! Requires the `motion` feature and Rake's patched `gpui-pre`. See `docs/motion.md`.
//!
//! ```no_run
//! use rake::gpui::{self, IntoElement, Styled};
//! use rake::motion::{self, Spring, Target};
//!
//! fn panel() -> impl IntoElement {
//!     motion::layer("panel", gpui::div().size(gpui::px(120.)))
//! }
//!
//! fn slide_away(window: &mut gpui::Window, cx: &gpui::App) {
//!     let target = Target::offset(gpui::px(240.), gpui::px(0.)).with_opacity(0.0);
//!
//!     motion::animate("panel", target, Spring::SMOOTH, window, cx);
//! }
//! ```

use crate::gpui::{
    App, ElementId, IntoElement, Pixels, PresentationLayer, PresentationTarget, SpringConfig,
    Window, point, presentation_layer, px,
};

/// An element whose painted content can animate without rendering its view.
pub type Layer = PresentationLayer;

/// The visual state of a layer relative to its laid-out position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    x: Pixels,
    y: Pixels,
    opacity: f32,
}

impl Target {
    /// The laid-out position at full opacity.
    pub const REST: Self = Self {
        x: px(0.),
        y: px(0.),
        opacity: 1.,
    };

    /// Creates a fully opaque target offset from the laid-out position.
    pub const fn offset(x: Pixels, y: Pixels) -> Self {
        Self { x, y, opacity: 1. }
    }

    /// Returns this target with `opacity`, which GPUI clamps from `0.0` to `1.0`.
    #[must_use]
    pub const fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }

    /// Returns the horizontal offset.
    pub const fn x(self) -> Pixels {
        self.x
    }

    /// Returns the vertical offset.
    pub const fn y(self) -> Pixels {
        self.y
    }

    /// Returns the opacity.
    pub const fn opacity(self) -> f32 {
        self.opacity
    }
}

impl Default for Target {
    fn default() -> Self {
        Self::REST
    }
}

impl From<Target> for PresentationTarget {
    fn from(target: Target) -> Self {
        Self {
            offset: point(target.x, target.y),
            opacity: target.opacity,
        }
    }
}

impl From<PresentationTarget> for Target {
    fn from(target: PresentationTarget) -> Self {
        Self {
            x: target.offset.x,
            y: target.offset.y,
            opacity: target.opacity,
        }
    }
}

/// The physical spring that drives a layer toward its target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring(SpringConfig);

impl Spring {
    /// A quick spring for direct responses to input.
    pub const SNAPPY: Self = Self::new(420., 36., 1.);

    /// A critically damped spring that arrives without overshoot.
    pub const SMOOTH: Self = Self::new(170., 26., 1.);

    /// An underdamped spring that visibly overshoots its target.
    pub const BOUNCY: Self = Self::new(260., 16., 1.);

    /// Creates a spring from its stiffness, damping and mass.
    ///
    /// A layer moves to its target immediately unless all three values are finite and positive.
    pub const fn new(stiffness: f32, damping: f32, mass: f32) -> Self {
        Self(SpringConfig::new(stiffness, damping, mass))
    }
}

impl Default for Spring {
    fn default() -> Self {
        Self::SMOOTH
    }
}

/// Wraps `child` in a layer identified by `id`, which must be unique within its window.
pub fn layer(id: impl Into<ElementId>, child: impl IntoElement) -> Layer {
    presentation_layer(id, child)
}

/// Springs a layer toward `target`, carrying its velocity across interruptions.
///
/// The layer moves immediately when the application has reduced motion enabled.
pub fn animate(
    id: impl Into<ElementId>,
    target: Target,
    spring: Spring,
    window: &mut Window,
    cx: &App,
) {
    window.animate_presentation_layer(id, target.into(), spring.0, cx);
}

/// Moves a layer to `target` immediately, cancelling any running spring.
pub fn jump(id: impl Into<ElementId>, target: Target, window: &mut Window) {
    window.set_presentation_layer(id, target.into());
}

/// Returns the state a layer currently shows.
pub fn value(id: impl Into<ElementId>, window: &Window) -> Target {
    window.presentation_layer_value(id).into()
}

/// Returns whether a layer's spring is still moving.
pub fn is_animating(id: impl Into<ElementId>, window: &Window) -> bool {
    window.is_presentation_layer_animating(id)
}
