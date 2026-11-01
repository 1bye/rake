//! Real AppKit controls in a Rake window. Run with `--features native-components`.

#[cfg(target_os = "macos")]
mod demo {
    use rake::gpui::{self, Context, IntoElement, Render, Window, prelude::*};
    use rake::native::macos::{Button, ButtonStyle, NativeError, SegmentedControl};

    struct Demo {
        clicks: usize,
        selected: usize,
        visible: bool,
        save: Button,
        toggle: Button,
        segments: SegmentedControl,
    }

    impl Demo {
        fn new(cx: &mut Context<Self>) -> Result<Self, NativeError> {
            let save = Button::new("Native glass button", ButtonStyle::LiquidGlass)?;
            let entity = cx.weak_entity();

            save.on_click(cx, move |cx| {
                entity
                    .update(cx, |this, cx| {
                        this.clicks += 1;
                        cx.notify();
                    })
                    .ok();
            });
            let toggle = Button::new("Show / hide controls", ButtonStyle::Push)?;
            let entity = cx.weak_entity();

            toggle.on_click(cx, move |cx| {
                entity
                    .update(cx, |this, cx| {
                        this.visible = !this.visible;
                        cx.notify();
                    })
                    .ok();
            });
            let segments = SegmentedControl::new(&["Overview", "Activity", "Settings"])?;
            let entity = cx.weak_entity();

            segments.on_change(cx, move |index, cx| {
                entity
                    .update(cx, |this, cx| {
                        this.selected = index;
                        cx.notify();
                    })
                    .ok();
            });

            Ok(Self {
                clicks: 0,
                selected: 0,
                visible: true,
                save,
                toggle,
                segments,
            })
        }
    }

    impl Render for Demo {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            gpui::div()
                .size_full()
                .p_8()
                .flex()
                .flex_col()
                .gap_4()
                .bg(gpui::rgb(0x182330))
                .text_color(gpui::rgb(0xffffff))
                .child("Rake · native macOS components")
                .child(format!(
                    "Native button clicks: {} · Selected segment: {}",
                    self.clicks, self.selected
                ))
                .child(self.toggle.element().w(gpui::px(220.)).h(gpui::px(32.)))
                .when(self.visible, |root| {
                    root.child(self.save.element().w(gpui::px(220.)).h(gpui::px(36.)))
                        .child(self.segments.element().w(gpui::px(320.)).h(gpui::px(32.)))
                })
                .child("AppKit draws these controls and handles their input and accessibility.")
        }
    }

    pub(super) fn run() -> Result<(), rake::AppError> {
        rake::App::new("com.example.rake.native")
            .root(|_, _, cx| {
                cx.new(|cx| Demo::new(cx).expect("native controls require the AppKit main thread"))
                    .into()
            })
            .run()
    }
}

fn main() {
    #[cfg(target_os = "macos")]
    demo::run().expect("native example starts");
    #[cfg(not(target_os = "macos"))]
    eprintln!("The native-components example currently requires macOS.");
}
