//! Shared Inspector colors, typography and small vector controls.

use rake::gpui::{prelude::*, *};

pub(crate) const CODE_FONT: &str = if cfg!(target_family = "wasm") {
    "Lilex"
} else if cfg!(target_os = "macos") {
    "Menlo"
} else {
    "monospace"
};

#[derive(Clone, Copy)]
pub(crate) struct Palette {
    pub background: u32,
    pub bar: u32,
    pub field: u32,
    pub line: u32,
    pub guide: u32,
    pub ink: u32,
    pub muted: u32,
    pub accent: u32,
    pub tag: u32,
    pub attribute: u32,
    pub value: u32,
    pub property: u32,
    pub selection: u32,
    pub hover: u32,
}

impl Palette {
    pub fn new(light: bool) -> Self {
        if light {
            Self {
                background: 0xfdfcfb,
                bar: 0xf0f0ed,
                field: 0xe8e8e5,
                line: 0xdedfd9,
                guide: 0xecece7,
                ink: 0x292a28,
                muted: 0x777b74,
                accent: 0x46634f,
                tag: 0x881280,
                attribute: 0x994500,
                value: 0x1a1aa6,
                property: 0xd33422,
                selection: 0xdce8f5,
                hover: 0xf0f0ed,
            }
        } else {
            Self {
                background: 0x202124,
                bar: 0x292a2d,
                field: 0x35363a,
                line: 0x3c4043,
                guide: 0x303236,
                ink: 0xe8eaed,
                muted: 0x9aa0a6,
                accent: 0xa8c7fa,
                tag: 0xc58af9,
                attribute: 0xd4a469,
                value: 0x9dc7f5,
                property: 0xf28b82,
                selection: 0x354b69,
                hover: 0x2c2e32,
            }
        }
    }
}

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
}

pub(crate) fn icon(kind: Icon, color: u32) -> impl IntoElement {
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
            };

            for line in lines {
                path.move_to(point_at(line[0].0, line[0].1));

                for &(x, y) in &line[1..] {
                    path.line_to(point_at(x, y));
                }
            }

            if let Ok(path) = path.build() {
                window.paint_path(path, rgb(color));
            }
        },
    )
    .size(px(16.))
    .flex_shrink_0()
}
