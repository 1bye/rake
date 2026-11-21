use rake::gpui::{App, Hsla, IntoElement, ParentElement, Styled, Window, div, px};
use rake::motion::{self, Spring, Target};

const BAR_LAYER: &str = "overview-bar";
const COLLAPSED_WIDTH: f32 = 72.0;
const EXPANDED_WIDTH: f32 = 312.0;

fn target(expanded: bool) -> Target {
    if expanded {
        Target::REST
    } else {
        Target::offset(px(COLLAPSED_WIDTH - EXPANDED_WIDTH), px(0.0))
    }
}

pub(crate) fn initialize(window: &mut Window) {
    motion::jump(BAR_LAYER, target(false), window);
}

pub(crate) fn retarget(expanded: bool, window: &mut Window, cx: &App) {
    motion::animate(BAR_LAYER, target(expanded), Spring::SMOOTH, window, cx);
}

pub(crate) fn bar(color: Hsla) -> impl IntoElement {
    div()
        .w(px(EXPANDED_WIDTH))
        .overflow_hidden()
        .child(motion::layer(
            BAR_LAYER,
            div().w_full().h_2().rounded_full().bg(color),
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_preserve_the_bar_widths_inside_the_clip() {
        assert_eq!(px(EXPANDED_WIDTH) + target(false).x(), px(72.0));
        assert_eq!(px(EXPANDED_WIDTH) + target(true).x(), px(312.0));
    }
}
