//! Covers: the real example's complete composed text, whitespace, and conditional content.
//! Real: Counter, components, retained entities, overlays, GPUI layout and text composition.
//! Doubles: GPUI TestPlatform and a recording wrapper around NoopTextSystem.
//! Requires: view-test-support; no native window, GPU or installed fonts.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use rake::gpui::{self, PlatformTextSystem};

use super::{BumpRow, Counter, Increment, ToggleHistory, ToggleLocked, components};

#[derive(Default)]
struct RecordedText(Mutex<BTreeSet<String>>);

impl PlatformTextSystem for RecordedText {
    fn add_fonts(&self, fonts: Vec<Cow<'static, [u8]>>) -> gpui::Result<()> {
        gpui::NoopTextSystem.add_fonts(fonts)
    }

    fn all_font_names(&self) -> Vec<String> {
        gpui::NoopTextSystem.all_font_names()
    }

    fn font_id(&self, font: &gpui::Font) -> gpui::Result<gpui::FontId> {
        gpui::NoopTextSystem.font_id(font)
    }

    fn font_metrics(&self, font: gpui::FontId) -> gpui::FontMetrics {
        gpui::NoopTextSystem.font_metrics(font)
    }

    fn typographic_bounds(
        &self,
        font: gpui::FontId,
        glyph: gpui::GlyphId,
    ) -> gpui::Result<gpui::Bounds<f32>> {
        gpui::NoopTextSystem.typographic_bounds(font, glyph)
    }

    fn advance(&self, font: gpui::FontId, glyph: gpui::GlyphId) -> gpui::Result<gpui::Size<f32>> {
        gpui::NoopTextSystem.advance(font, glyph)
    }

    fn glyph_for_char(&self, font: gpui::FontId, ch: char) -> Option<gpui::GlyphId> {
        gpui::NoopTextSystem.glyph_for_char(font, ch)
    }

    fn glyph_raster_bounds(
        &self,
        params: &gpui::RenderGlyphParams,
    ) -> gpui::Result<gpui::Bounds<gpui::DevicePixels>> {
        gpui::NoopTextSystem.glyph_raster_bounds(params)
    }

    fn rasterize_glyph(
        &self,
        params: &gpui::RenderGlyphParams,
        bounds: gpui::Bounds<gpui::DevicePixels>,
    ) -> gpui::Result<(gpui::Size<gpui::DevicePixels>, Vec<u8>)> {
        gpui::NoopTextSystem.rasterize_glyph(params, bounds)
    }

    fn layout_line(
        &self,
        text: &str,
        size: gpui::Pixels,
        runs: &[gpui::FontRun],
    ) -> gpui::LineLayout {
        // Layout also measures candidate prefixes while finding an ellipsis.
        // Assert complete source text, without coupling the oracle to font widths.
        if !text.is_empty() && !text.ends_with('…') {
            self.0.lock().expect("record text").insert(text.to_owned());
        }

        gpui::NoopTextSystem.layout_line(text, size, runs)
    }

    fn recommended_rendering_mode(
        &self,
        font: gpui::FontId,
        size: gpui::Pixels,
    ) -> gpui::TextRenderingMode {
        gpui::NoopTextSystem.recommended_rendering_mode(font, size)
    }
}

#[derive(Clone, Copy, Debug)]
enum Scene {
    Initial,
    LightTheme,
    Counted,
    LockedHidden,
    Actions,
    Documents,
    SelectedDocument,
}

fn expected_text(scene: Scene) -> BTreeSet<String> {
    let count = if matches!(scene, Scene::Counted | Scene::LockedHidden) {
        "2"
    } else {
        "0"
    };

    let mut expected: BTreeSet<_> = include_str!("text/common.txt")
        .replace("{count}", count)
        .lines()
        .map(str::to_owned)
        .collect();

    let state = match scene {
        Scene::Counted => include_str!("text/counted.txt"),
        Scene::LockedHidden => include_str!("text/locked-hidden.txt"),
        _ => include_str!("text/initial.txt"),
    };

    expected.extend(state.lines().map(str::to_owned));

    if matches!(
        scene,
        Scene::Actions | Scene::Documents | Scene::SelectedDocument
    ) {
        expected.extend(include_str!("text/actions.txt").lines().map(str::to_owned));
    }

    if matches!(scene, Scene::Documents | Scene::SelectedDocument) {
        let documents = if matches!(scene, Scene::SelectedDocument) {
            include_str!("text/selected-document.txt")
        } else {
            include_str!("text/documents.txt")
        };

        expected.extend(documents.lines().map(str::to_owned));
    }

    expected
}

#[test]
fn example_complete_composed_text_matches_reviewed_fixtures() {
    for scene in [
        Scene::Initial,
        Scene::LightTheme,
        Scene::Counted,
        Scene::LockedHidden,
        Scene::Actions,
        Scene::Documents,
        Scene::SelectedDocument,
    ] {
        let text = Arc::new(RecordedText::default());
        let mut app = gpui::TestApp::with_text_system(text.clone());

        let mut window = app.open_window_with_options(
            gpui::WindowOptions {
                window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds {
                    origin: gpui::Point::default(),
                    size: gpui::size(gpui::px(1000.), gpui::px(1200.)),
                })),
                ..Default::default()
            },
            |window, cx| {
                let mut counter = Counter::new(window, cx);

                if matches!(scene, Scene::LightTheme) {
                    counter.toggle_theme(&super::ToggleTheme, window, cx);
                }

                if matches!(scene, Scene::Counted | Scene::LockedHidden) {
                    counter.increment(&Increment, window, cx);
                    counter.increment(&Increment, window, cx);
                    counter.history[0].update(cx, |row, cx| row.bump(&BumpRow, window, cx));
                }

                if matches!(scene, Scene::LockedHidden) {
                    counter.toggle_locked(&ToggleLocked, window, cx);
                    counter.toggle_history(&ToggleHistory, window, cx);
                }

                if matches!(scene, Scene::SelectedDocument) {
                    counter
                        .documents
                        .update(cx, |documents, cx| documents.jump(4_999, window, cx));

                    window.focus(&counter.focus, cx);
                }

                counter
            },
        );

        window.draw();

        if matches!(
            scene,
            Scene::Actions | Scene::Documents | Scene::SelectedDocument
        ) {
            window.update(|counter, window, cx| assert!(counter.actions_popup.open(window, cx)));
            window.draw();
        }

        if matches!(scene, Scene::Documents | Scene::SelectedDocument) {
            window.update(|counter, window, cx| assert!(counter.documents_dialog.open(window, cx)));
            window.draw();
        }

        assert_eq!(
            *text.0.lock().expect("read recorded text"),
            expected_text(scene),
            "{scene:?}"
        );
    }
}

#[test]
fn tooltip_complete_text_matches_the_real_help_view() {
    let text = Arc::new(RecordedText::default());
    let mut app = gpui::TestApp::with_text_system(text.clone());
    let mut window = app.open_window(|_, cx| {
        rake::theme::install(cx, super::Colors::dark());
        components::ActionsHint
    });

    window.draw();

    assert_eq!(*text.0.lock().expect("read tooltip text"), BTreeSet::from([
        "Actions open an anchored popover; the document browser keeps its selection and virtual-list scroll.".to_owned(),
    ]));
}
