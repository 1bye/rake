//! Box geometry from measured bounds and the style used during request-layout.
//! GPUI does not expose Taffy's used percentage/auto edges or scrollbar metrics.

use gpui::{AbsoluteLength, DefiniteLength, Edges, Length, Pixels, Style};

use super::super::{BoxModel, Layout};

pub(super) fn capture(style: &Style, border: Layout, rem: Pixels, scale: f32) -> BoxModel {
    let mut model = BoxModel {
        border,
        padding: None,
        content: None,
        margin: None,
        unavailable: Vec::new(),
    };
    if style.display == gpui::Display::None {
        model
            .unavailable
            .push("Hidden layout has no inner or margin boxes".into());
        return model;
    }
    let absolute = |length: AbsoluteLength| {
        let device = f32::from(length.to_pixels(rem)) * scale;
        // Matches the pinned GPUI layout engine's half-toward-zero rounding.
        let value = (device.abs() - 0.5).ceil().copysign(device) / scale;
        value.is_finite().then_some(value)
    };
    let definite = |length| match length {
        DefiniteLength::Absolute(length) => absolute(length),
        DefiniteLength::Fraction(_) => None,
    };
    let margin = |length| match length {
        Length::Definite(length) => definite(length),
        Length::Auto => None,
    };
    let stroke = |length: AbsoluteLength| {
        let value = absolute(length)?;
        Some(if length.is_zero() {
            0.
        } else {
            value.max(1. / scale)
        })
    };
    model.padding =
        edges(&style.border_widths, stroke).and_then(|edges| inset(border, edges, true));
    model.content = model.padding.and_then(|padding| {
        edges(&style.padding, definite).and_then(|edges| inset(padding, edges, true))
    });
    model.margin = edges(&style.margin, margin)
        .and_then(|edges| inset(border, edges.map(|edge| -edge), false));
    if model.padding.is_none() {
        model
            .unavailable
            .push("Padding box has invalid border geometry".into());
    }
    if model.content.is_none() {
        model
            .unavailable
            .push("Content box needs used percentage padding or valid absolute edges".into());
    }
    if model.margin.is_none() {
        model
            .unavailable
            .push("Margin box needs used auto/percentage margins or valid absolute edges".into());
    }
    if !style.scrollbar_width.is_zero()
        && (style.overflow.x == gpui::Overflow::Scroll
            || style.overflow.y == gpui::Overflow::Scroll)
    {
        model.content = None;
        model
            .unavailable
            .push("Content box needs used scrollbar metrics".into());
    }
    model
}

fn edges<T: Copy + std::fmt::Debug + Default + PartialEq>(
    edges: &Edges<T>,
    resolve: impl Fn(T) -> Option<f32>,
) -> Option<Edges<f32>> {
    Some(Edges {
        top: resolve(edges.top)?,
        right: resolve(edges.right)?,
        bottom: resolve(edges.bottom)?,
        left: resolve(edges.left)?,
    })
}

fn inset(bounds: Layout, edges: Edges<f32>, clamp: bool) -> Option<Layout> {
    let mut bounds = Layout {
        x: bounds.x + edges.left,
        y: bounds.y + edges.top,
        width: bounds.width - edges.left - edges.right,
        height: bounds.height - edges.top - edges.bottom,
    };
    if ![bounds.x, bounds.y, bounds.width, bounds.height]
        .iter()
        .all(|value| value.is_finite())
    {
        return None;
    }
    if clamp {
        bounds.width = bounds.width.max(0.);
        bounds.height = bounds.height.max(0.);
    }
    Some(bounds)
}

#[cfg(test)]
mod tests {
    // Real pinned GPUI styles/units; no renderer. Window offsets are already measured.
    use super::*;

    #[test]
    fn asymmetric_absolute_edges_rems_and_device_rounding() {
        let style = Style {
            border_widths: Edges {
                top: gpui::px(1.25).into(),
                right: gpui::px(2.).into(),
                bottom: gpui::px(3.).into(),
                left: gpui::px(4.).into(),
            },
            padding: Edges {
                top: gpui::rems(0.5).into(),
                right: gpui::px(6.).into(),
                bottom: gpui::px(7.).into(),
                left: gpui::px(8.).into(),
            },
            margin: Edges {
                top: gpui::px(10.).into(),
                right: gpui::px(-2.).into(),
                bottom: gpui::px(12.).into(),
                left: gpui::px(13.).into(),
            },
            ..Style::default()
        };
        let border = Layout {
            x: 41.,
            y: 52.,
            width: 100.,
            height: 80.,
        };
        let boxes = capture(&style, border, gpui::px(16.), 2.);

        assert_eq!(
            boxes.padding,
            Some(Layout {
                x: 45.,
                y: 53.,
                width: 94.,
                height: 76.
            })
        );
        assert_eq!(
            boxes.content,
            Some(Layout {
                x: 53.,
                y: 61.,
                width: 80.,
                height: 61.
            })
        );
        assert_eq!(
            boxes.margin,
            Some(Layout {
                x: 28.,
                y: 42.,
                width: 111.,
                height: 102.
            })
        );
        assert!(boxes.unavailable.is_empty());
    }

    #[test]
    fn unknown_used_edges_never_duplicate_border_bounds() {
        let mut style = Style::default();
        style.padding.left = DefiniteLength::Fraction(0.1);
        style.margin.right = Length::Auto;
        let boxes = capture(
            &style,
            Layout {
                x: 0.,
                y: 0.,
                width: 100.,
                height: 80.,
            },
            gpui::px(16.),
            1.,
        );
        assert!(boxes.padding.is_some());
        assert!(boxes.content.is_none());
        assert!(boxes.margin.is_none());
        assert_eq!(boxes.unavailable.len(), 2);
    }

    #[test]
    fn hairline_borders_negative_margins_and_invalid_edges() {
        let mut style = Style::default();
        style.border_widths.left = gpui::px(0.1).into();
        style.margin.left = gpui::px(-150.).into();
        let border = Layout {
            x: 10.,
            y: 20.,
            width: 100.,
            height: 80.,
        };
        let boxes = capture(&style, border, gpui::px(16.), 2.);
        assert_eq!(
            boxes.padding.unwrap().x,
            10.5,
            "nonzero border strokes occupy at least one device pixel"
        );
        assert_eq!(
            boxes.margin.unwrap().width,
            -50.,
            "negative margins preserve signed edges"
        );
        style.padding.top = gpui::px(f32::NAN).into();
        let boxes = capture(&style, border, gpui::px(16.), 2.);
        assert!(boxes.content.is_none());
        assert!(boxes.padding.is_some());
    }
}
