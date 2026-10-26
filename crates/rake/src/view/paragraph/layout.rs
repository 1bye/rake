//! Measures complete styled runs and truncates only at grapheme boundaries that really fit.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    App, AvailableSpace, Bounds, LayoutId, Pixels, SharedString, Size, TextAlign, TextLayout,
    TextOverflow, TextRun, Window, WrappedLine,
};

use unicode_segmentation::UnicodeSegmentation as _;

/// Measured GPUI lines shared by layout and paint for one paragraph frame.
#[doc(hidden)]
#[derive(Default)]
pub struct ParagraphLayout {
    pub(super) lines: Vec<WrappedLine>,
    line_height: Pixels,
    align: TextAlign,
}

pub(super) fn request_layout(
    text: SharedString,
    runs: Vec<TextRun>,
    state: Rc<RefCell<ParagraphLayout>>,
    window: &mut Window,
) -> LayoutId {
    let style = window.text_style();
    let font_size = style.font_size.to_pixels(window.rem_size());

    let line_height = window.pixel_snap(
        style
            .line_height
            .to_pixels(font_size.into(), window.rem_size()),
    );

    window.request_measured_layout(Default::default(), move |known, available, window, _| {
        let width = known.width.or(match available.width {
            AvailableSpace::Definite(width) => Some(width.max(gpui::px(0.))),
            _ => None,
        });

        let wrap = TextLayout::evaluate_wrap_width(&style.white_space, known, available);

        let affix = match &style.text_overflow {
            Some(TextOverflow::Truncate(affix)) => Some(affix.as_ref()),
            _ => None,
        };

        let limit = style
            .line_clamp
            .map(|lines| lines.max(1))
            .or_else(|| affix.filter(|_| width.is_some()).map(|_| 1));

        let limits = TextLimits {
            wrap,
            width: affix.and(width),
            lines: limit,
            affix: affix.unwrap_or(""),
        };

        let lines = fitted_text(&text, &runs, font_size, &limits, window);
        let mut size: Size<Pixels> = Size::default();

        for line in &lines {
            let line_size = line.size(line_height);

            size.width = size.width.max(line_size.width).ceil();
            size.height += line_size.height;
        }

        state.replace(ParagraphLayout {
            lines,
            line_height,
            align: style.text_align,
        });

        size
    })
}

fn shape(
    text: SharedString,
    runs: &[TextRun],
    font_size: Pixels,
    wrap: Option<Pixels>,
    window: &Window,
) -> Vec<WrappedLine> {
    match window
        .text_system()
        .shape_text(text, font_size, runs, wrap, None)
    {
        Ok(lines) => lines.into_vec(),
        Err(error) => {
            tracing::warn!(%error, "paragraph text shaping failed");

            Vec::new()
        }
    }
}

struct TextLimits<'a> {
    wrap: Option<Pixels>,
    width: Option<Pixels>,
    lines: Option<usize>,
    affix: &'a str,
}

fn fits(lines: &[WrappedLine], limits: &TextLimits<'_>) -> bool {
    limits.lines.is_none_or(|limit| {
        lines
            .iter()
            .map(|line| line.wrap_boundaries.len() + 1)
            .sum::<usize>()
            <= limit
    }) && limits
        .width
        .is_none_or(|width| lines.iter().all(|line| line.width() <= width))
}

fn prefix_runs(runs: &[TextRun], end: usize, affix_len: usize) -> Vec<TextRun> {
    let mut result = Vec::new();
    let mut start = 0;

    for run in runs {
        if start >= end {
            break;
        }

        let mut run = run.clone();

        run.len = run.len.min(end - start);
        start += run.len;
        result.push(run);
    }

    if affix_len > 0 {
        if let Some(last) = result.last_mut() {
            last.len += affix_len;
        } else if let Some(first) = runs.first() {
            let mut run = first.clone();

            run.len = affix_len;
            result.push(run);
        }
    }

    result
}

fn fitted_text(
    text: &SharedString,
    runs: &[TextRun],
    font_size: Pixels,
    limits: &TextLimits<'_>,
    window: &Window,
) -> Vec<WrappedLine> {
    let lines = shape(text.clone(), runs, font_size, limits.wrap, window);

    if fits(&lines, limits) {
        return lines;
    }

    let boundaries: Vec<_> = text
        .grapheme_indices(true)
        .map(|(index, _)| index)
        .collect();

    let mut low = 0;
    let mut high = boundaries.len();
    let mut best = None;

    // Every retained candidate was shaped with its actual styles, fonts and emoji.
    // Binary search avoids reshaping every prefix of a long paragraph.
    while low < high {
        let middle = low + (high - low) / 2;
        let prefix = text[..boundaries[middle]].trim_end();
        let candidate: SharedString = format!("{prefix}{}", limits.affix).into();
        let candidate_runs = prefix_runs(runs, prefix.len(), limits.affix.len());
        let lines = shape(candidate, &candidate_runs, font_size, limits.wrap, window);

        if fits(&lines, limits) {
            best = Some(lines);
            low = middle + 1;
        } else {
            high = middle;
        }
    }

    best.unwrap_or_else(|| shape(SharedString::default(), &[], font_size, limits.wrap, window))
}

pub(super) fn paint(
    layout: &ParagraphLayout,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let mut origin = bounds.origin;

    for line in &layout.lines {
        if let Err(error) = line
            .paint_background(
                origin,
                layout.line_height,
                layout.align,
                Some(bounds),
                window,
                cx,
            )
            .and_then(|_| {
                line.paint(
                    origin,
                    layout.line_height,
                    layout.align,
                    Some(bounds),
                    window,
                    cx,
                )
            })
        {
            tracing::warn!(%error, "paragraph text painting failed");
        }

        origin.y += line.size(layout.line_height).height;
    }
}
