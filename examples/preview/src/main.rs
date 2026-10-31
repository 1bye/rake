//! Native preview fixture: real behind-window blur, an atomic click and a scroll container.

use rake::gpui::prelude::*;
use rake::gpui::{self, Context, IntoElement, Render, ScrollHandle, Window, WindowOptions};

// Native regression fixture: fully cover the inspected window without hiding it.
#[cfg(feature = "devtools")]
struct Cover;

#[cfg(feature = "devtools")]
impl Render for Cover {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::div().size_full().bg(gpui::rgb(0x182330))
    }
}

struct Preview {
    count: usize,
    blurred: bool,
    scroll: ScrollHandle,
}

impl Render for Preview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.count;
        let blurred = self.blurred;

        rake::view! {
            <div id="preview-root" class="size-full flex flex-col p-6 gap-4"
                style={|s| s.bg(gpui::rgba(0x15233266)).text_color(gpui::rgb(0xffffff))}>
                <h1 class="text-2xl">"Native macOS preview"</h1>
                <p>"The translucent area uses the real macOS window backdrop."</p>
                <div class="flex gap-3">
                    <button id="preview-increment" class="px-4 py-2 rounded-md" style={|s| s.bg(gpui::rgb(0x2857a4))}
                        on:click={cx.listener(|this, _, _, cx| { this.count += 1; cx.notify(); })}>
                        {format!("Clicks: {count}")}
                    </button>
                    <button id="preview-blur" class="px-4 py-2 rounded-md" style={|s| s.bg(gpui::rgb(0x2857a4))}
                        on:click={cx.listener(|this, _, window, cx| {
                            this.blurred = !this.blurred;
                            window.set_background_appearance(if this.blurred {
                                gpui::WindowBackgroundAppearance::Blurred
                            } else { gpui::WindowBackgroundAppearance::Opaque });
                            cx.notify();
                        })}>
                        {if blurred { "Backdrop: native blur" } else { "Backdrop: opaque" }}
                    </button>
                </div>
                <div id="preview-scroll" class="flex-1 min-h-0 overflow-y-scroll flex flex-col gap-2"
                    scroll={&self.scroll}>
                    for row in 1..=40 {
                        <p id={format!("preview-row-{row}")} class="p-3 rounded-md"
                            style={|s| s.bg(gpui::rgba(0x30476688))}>
                            {format!("Scrollable row {row}")}
                        </p>
                    }
                </div>
            </div>
        }
    }
}

fn main() -> Result<(), rake::AppError> {
    #[cfg(feature = "devtools")]
    if std::env::var_os("RAKE_PREVIEW_TEST_COVER").as_deref() == Some(std::ffi::OsStr::new("1")) {
        return rake::App::new("com.example.rake.preview.cover")
            .window_options(WindowOptions {
                kind: gpui::WindowKind::Floating,
                window_bounds: Some(gpui::WindowBounds::Maximized(gpui::Bounds {
                    origin: gpui::point(gpui::px(60.), gpui::px(80.)),
                    size: gpui::size(gpui::px(800.), gpui::px(640.)),
                })),
                ..Default::default()
            })
            .root(|_, _, cx| cx.new(|_| Cover).into())
            .run();
    }

    rake::App::new("com.example.rake.preview")
        .control(rake::control::ControlConfig::FromEnvironment)
        .window_options(WindowOptions {
            window_background: gpui::WindowBackgroundAppearance::Blurred,
            window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds {
                origin: gpui::point(gpui::px(80.), gpui::px(100.)),
                size: gpui::size(gpui::px(760.), gpui::px(600.)),
            })),
            ..Default::default()
        })
        .root(|_, _, cx| {
            cx.new(|_| Preview {
                count: 0,
                blurred: true,
                scroll: ScrollHandle::new(),
            })
            .into()
        })
        .run()
}
