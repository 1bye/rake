//! Shared Inspector colors, typography and small vector controls.

use rake::gpui::{prelude::*, *};

pub(crate) const CODE_FONT: &str = if cfg!(target_family = "wasm") {
    "Lilex"
} else if cfg!(target_os = "macos") {
    "Menlo"
} else {
    "monospace"
};

#[derive(Clone, Copy, rake::Theme)]
pub(crate) struct Palette {
    pub matched: Rgba,
    pub warning: Rgba,
    pub warning_surface: Rgba,
    pub status_success: Rgba,
    pub code: &'static str,
    pub background: Rgba,
    pub bar: Rgba,
    pub field: Rgba,
    pub line: Rgba,
    pub guide: Rgba,
    pub ink: Rgba,
    pub muted: Rgba,
    pub accent: Rgba,
    pub tag: Rgba,
    pub attribute: Rgba,
    pub value: Rgba,
    pub property: Rgba,
    pub selection: Rgba,
    pub inactive_selection: Rgba,
    pub focus: Rgba,
    pub hover: Rgba,
}

impl Palette {
    pub fn new(light: bool) -> Self {
        if light {
            Self {
                code: CODE_FONT,
                matched: rgb(0xffedb1),
                warning: rgb(0x826113),
                warning_surface: rgb(0xfff6d9),
                status_success: rgb(0x6d9668),
                background: rgb(0xfdfcfb),
                bar: rgb(0xf0f0ed),
                field: rgb(0xe8e8e5),
                line: rgb(0xdedfd9),
                guide: rgb(0xecece7),
                ink: rgb(0x292a28),
                muted: rgb(0x777b74),
                accent: rgb(0x46634f),
                tag: rgb(0x881280),
                attribute: rgb(0x994500),
                value: rgb(0x1a1aa6),
                property: rgb(0xd33422),
                selection: rgb(0xdce8f5),
                inactive_selection: rgb(0xe7e7e4),
                focus: rgb(0x1a73e8),
                hover: rgb(0xf0f0ed),
            }
        } else {
            Self {
                code: CODE_FONT,
                matched: rgb(0x614c2b),
                warning: rgb(0xfdd663),
                warning_surface: rgb(0x3b3321),
                status_success: rgb(0x6d9668),
                background: rgb(0x202124),
                bar: rgb(0x292a2d),
                field: rgb(0x35363a),
                line: rgb(0x3c4043),
                guide: rgb(0x303236),
                ink: rgb(0xe8eaed),
                muted: rgb(0x9aa0a6),
                accent: rgb(0xa8c7fa),
                tag: rgb(0xc58af9),
                attribute: rgb(0xd4a469),
                value: rgb(0x9dc7f5),
                property: rgb(0xf28b82),
                selection: rgb(0x354b69),
                inactive_selection: rgb(0x3a3d42),
                focus: rgb(0x8ab4f8),
                hover: rgb(0x2c2e32),
            }
        }
    }
}

#[rake::style("focus-visible:bg-selection focus-visible:text-ink")]
pub(crate) fn control();

#[derive(Clone, Copy)]
pub(crate) enum Icon {
    Right,
    Down,
    Search,
    Refresh,
    Pause,
    Play,
    Theme,
    Code,
    Reveal,
}

pub(crate) fn icon(kind: Icon, color: Rgba) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let mut path = if matches!(kind, Icon::Right | Icon::Down | Icon::Play | Icon::Pause) {
                PathBuilder::fill()
            } else {
                PathBuilder::stroke(px(1.3))
            };

            let origin = bounds.origin
                + point(
                    (bounds.size.width - px(16.)) / 2.,
                    (bounds.size.height - px(16.)) / 2.,
                );

            let point_at = |x, y| origin + point(px(x), px(y));

            let lines: &[&[(f32, f32)]] = match kind {
                Icon::Right | Icon::Play => &[&[(5., 3.), (11., 8.), (5., 13.), (5., 3.)]],
                Icon::Down => &[&[(3., 5.), (13., 5.), (8., 11.), (3., 5.)]],
                Icon::Pause => &[
                    &[(4., 3.), (6., 3.), (6., 13.), (4., 13.), (4., 3.)],
                    &[(10., 3.), (12., 3.), (12., 13.), (10., 13.), (10., 3.)],
                ],
                Icon::Search => &[
                    &[(10.5, 10.5), (14., 14.)],
                    &[
                        (7., 2.),
                        (10.5, 3.5),
                        (12., 7.),
                        (10.5, 10.5),
                        (7., 12.),
                        (3.5, 10.5),
                        (2., 7.),
                        (3.5, 3.5),
                        (7., 2.),
                    ],
                ],
                Icon::Refresh => &[
                    &[
                        (13., 6.),
                        (11., 3.),
                        (7., 2.),
                        (3., 4.),
                        (2., 8.),
                        (4., 12.),
                        (8., 14.),
                        (12., 12.),
                    ],
                    &[(13., 2.), (13., 6.), (9., 6.)],
                ],
                Icon::Theme => &[&[
                    (8., 2.),
                    (12., 3.5),
                    (14., 8.),
                    (12., 12.5),
                    (8., 14.),
                    (4., 12.5),
                    (2., 8.),
                    (4., 3.5),
                    (8., 2.),
                    (8., 14.),
                ]],
                Icon::Code => &[
                    &[(6., 4.), (2., 8.), (6., 12.)],
                    &[(10., 4.), (14., 8.), (10., 12.)],
                ],
                Icon::Reveal => &[
                    &[(6., 2.), (2., 2.), (2., 6.)],
                    &[(10., 2.), (14., 2.), (14., 6.)],
                    &[(2., 10.), (2., 14.), (6., 14.)],
                    &[(14., 10.), (14., 14.), (10., 14.)],
                    &[(5., 8.), (11., 8.)],
                    &[(8., 5.), (8., 11.)],
                ],
            };

            for line in lines {
                path.move_to(point_at(line[0].0, line[0].1));

                for &(x, y) in &line[1..] {
                    path.line_to(point_at(x, y));
                }
            }

            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size(px(16.))
    .flex_shrink_0()
}
