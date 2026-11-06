//! Runnable spike; default workspace builds do not activate the GPU experiment.
#![cfg(all(target_os = "macos", feature = "fast"))]

use rake::{
    gpui::{self, prelude::*, px, rgb},
    overlay::{
        AnchorState, Dialog, OverlayAnchor, OverlayHost, OverlayHostState, OverlayState, Popover,
    },
    surface::GpuSurface,
};

use web_time::Instant;

mod input_layer;
mod producer;

struct Demo {
    preview: Option<GpuSurface>,
    _producer: Option<producer::Producer>,
    mounted: bool,
    hidden: bool,
    busy: bool,
    scroll: gpui::ScrollHandle,
    focus: gpui::FocusHandle,
    pointers: u64,
    keys: u64,
    _input_activity: gpui::Subscription,
    ui_focus: gpui::FocusHandle,
    overlays: OverlayHostState,
    anchor: AnchorState,
    popup: OverlayState,
    dialog: OverlayState,
    frames: u64,
    render_ms: f64,
    started: Instant,
    last_frame: Instant,
    frame_intervals_ms: f64,
}

impl Demo {
    fn new(window: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> Self {
        let overlays = OverlayHostState::new(cx);
        let no_surface = std::env::args().any(|argument| argument == "--baseline");

        let preview = (!no_surface)
            .then(|| GpuSurface::new(window, cx).expect("surface host initialization"));

        let focus = cx.focus_handle();

        window.focus(&focus, cx);
        let popup = overlays.popover(cx);
        let dialog = overlays.dialog(cx);

        let producer = preview.as_ref().map(|preview| {
            producer::Producer::start(
                preview,
                window,
                cx,
                focus.clone(),
                dialog.clone(),
                popup.clone(),
            )
        });

        let input_activity = cx.observe_window_activation(window, |this, window, _| {
            if !window.is_window_active()
                && let Some(producer) = &this._producer
            {
                producer.cancel(window);
            }
        });

        Self {
            preview,
            _producer: producer,
            mounted: true,
            hidden: false,
            busy: false,
            scroll: gpui::ScrollHandle::new(),
            focus,
            pointers: 0,
            keys: 0,
            _input_activity: input_activity,
            ui_focus: cx.focus_handle().tab_stop(true).tab_index(1),
            anchor: overlays.anchor(),
            popup,
            dialog,
            overlays,
            frames: 0,
            render_ms: 0.,
            started: Instant::now(),
            last_frame: Instant::now(),
            frame_intervals_ms: 0.,
        }
    }
}

impl Render for Demo {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let begin = Instant::now();

        self.frames += 1;
        self.frame_intervals_ms += begin.duration_since(self.last_frame).as_secs_f64() * 1000.;
        self.last_frame = begin;
        let entity = cx.entity().downgrade();

        window.on_next_frame(move |_, cx| {
            let _ = entity.update(cx, |_, cx| cx.notify());
        });
        window.request_animation_frame();
        let open_dialog = self.dialog.clone();
        let open_popup = self.popup.clone();
        let close_dialog = self.dialog.clone();

        let status = self
            .preview
            .as_ref()
            .map(GpuSurface::state)
            .unwrap_or_default();

        let slot = if self.mounted {
            self.preview.as_ref().map(|preview| {
                preview
                    .element()
                    .w_full()
                    .h(px(380.))
                    .when(self.hidden, |element| element.invisible())
                    .into_any_element()
            })
        } else {
            None
        };

        let input_layer = self
            ._producer
            .as_ref()
            .map(|producer| producer.input_layer(self.mounted && !self.hidden, window, cx));

        let controls = gpui::div().flex().gap_3().children([
            control("Hide / show")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.hidden = !this.hidden;
                    cx.notify();
                }))
                .into_any_element(),
            control("Remove / remount")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.mounted = !this.mounted;
                    cx.notify();
                }))
                .into_any_element(),
            control("Busy UI")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.busy = !this.busy;
                    cx.notify();
                }))
                .into_any_element(),
            control("Dialog")
                .on_click(move |_, window, cx| {
                    open_dialog.open(window, cx);
                })
                .into_any_element(),
            OverlayAnchor {
                state: self.anchor.clone(),
                children: vec![
                    control("Popover")
                        .on_click(move |_, window, cx| {
                            open_popup.open(window, cx);
                        })
                        .into_any_element(),
                ],
            }
            .into_any_element(),
        ]);

        let controls = controls.child(
            control("UI focus")
                .track_focus(&self.ui_focus)
                .when(self.ui_focus.is_focused(window), |element| {
                    element.bg(rgb(0x287d70))
                })
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        window.focus(&this.ui_focus, cx);
                        cx.notify();
                    }),
                ),
        );

        let controls = controls.children(self._producer.as_ref().map(|producer| {
            control("Text input")
                .when(producer.input.text_mode(), |element| {
                    element.bg(rgb(0x287d70))
                })
                .on_click(cx.listener(|this, _, window, cx| {
                    if let Some(producer) = &this._producer {
                        producer.input.toggle_text_mode(window);
                        cx.notify();
                    }
                }))
        }));

        let body = gpui::div()
            .id("surface-demo")
            .size_full()
            .bg(rgb(0x17212c))
            .text_color(rgb(0xeaf1f7))
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .track_focus(&self.focus)
            .on_key_down(
                cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                    if this._producer.as_ref().is_some_and(|producer| producer.input.accepts_text(window)) {
                        return;
                    }

                    this.keys += 1;

                    #[cfg(feature = "devtools")]
                    if event.keystroke.key == "m" {
                        window.reset_debug_frame_overlay_stats();
                    }

                    if event.keystroke.key == "h" {
                        this.hidden = !this.hidden;
                    }

                    if event.keystroke.key == "r" {
                        this.mounted = !this.mounted;
                    }

                    if event.keystroke.key == "b" {
                        this.busy = !this.busy;
                    }

                    cx.notify();
                }),
            )
            .child(if self.preview.is_some() {
                "GPU surface | Native composition"
            } else {
                "GPU surface | Baseline"
            })
            .child(controls)
            .child(format!(
                "{}x{} px @ {} | visible={} | pointer={} keys={} | UI busy={}",
                status.width_px,
                status.height_px,
                status.scale,
                status.visible,
                self.pointers,
                self.keys,
                self.busy
            ))
            .children(self._producer.as_ref().map(|producer| gpui::div().text_sm().child(producer.diagnostics())))
            .child(
                gpui::div()
                    .id("scroll-slot")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .rounded_xl()
                    .border_2()
                    .border_color(rgb(0x48b4bf))
                    .child(
                        gpui::div()
                            .id("tooltip-trigger")
                            .h(px(40.))
                            .child("Hover for tooltip. Viewport: click to focus, drag marker, wheel zoom, arrows move; UI focus / Tab leaves it.")
                            .tooltip(|_, cx| cx.new(|_| Tip).into()),
                    )
                    .child(
                        gpui::div()
                            .id("producer-input")
                            .relative()
                            .h(px(380.))
                            .w_full()
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(|this, _, window, cx| {
                                    this.pointers += 1;
                                    window.focus(&this.focus, cx);
                                    cx.notify();
                                }),
                            )
                            .children(slot)
                            .children(input_layer),
                    )
                    .children((0..if self.busy { 400 } else { 8 }).map(|index| {
                        gpui::div()
                            .h(px(25.))
                            .child(format!("GPUI row {index} | frame {}", self.frames))
                    })),
            )
            .child(format!(
                "UI renders={} mean render={:.3}ms mean interval={:.3}ms elapsed={:.1}s",
                self.frames,
                self.render_ms / self.frames as f64,
                self.frame_intervals_ms / self.frames as f64,
                self.started.elapsed().as_secs_f64()
            ));

        let view = rake::view! {
            <OverlayHost state={self.overlays.clone()} body={body.into_any_element()}>
                <Popover state={self.popup.clone()} anchor={self.anchor.clone()}>
                    <p>"GPUI popover above the renderer"</p>
                    <button on:click={move |_, _, _| { eprintln!("POPOVER_CLICK"); }}>"Popover input target"</button>
                </Popover>
                <Dialog state={self.dialog.clone()} title={"GPUI dialog above renderer".into()} footer={None}>
                    <p>"Tab, Escape and pointer input should stay in this GPUI dialog."</p>
                    <button on:click={move |_, window, cx| { close_dialog.close(window, cx); }}>"Close dialog"</button>
                </Dialog>
            </OverlayHost>
        };

        let view = {
            let ui_focus = self.ui_focus.clone();
            let dialog = self.dialog.clone();
            let popup = self.popup.clone();

            gpui::div()
                .size_full()
                .capture_key_down(move |event, window, _| {
                    if event.is_held {
                        eprintln!(
                            "NATIVE_REPEAT ui_focus={} dialog={} popover={}",
                            ui_focus.is_focused(window),
                            dialog.is_open(),
                            popup.is_open(),
                        );
                    }
                })
                .child(view)
        };

        self.render_ms += begin.elapsed().as_secs_f64() * 1000.;

        #[cfg(feature = "devtools")]
        if self.frames.is_multiple_of(300) {
            let timing = window.frame_duration_snapshot();
            let draw = &timing.draw_duration_histogram;
            let interval = &timing.present_interval_histogram;

            let measurement = format!(
                "GPUI_PROFILE draw_mean_ms={:.4} draw_p95_ms={:.4} interval_mean_ms={:.4} interval_p95_ms={:.4} n={} busy={}",
                draw.mean() / 1e6,
                draw.value_at_quantile(0.95) as f64 / 1e6,
                interval.mean() / 1e6,
                interval.value_at_quantile(0.95) as f64 / 1e6,
                draw.len(),
                self.busy
            );

            eprintln!("{measurement}");
        }

        if self.frames.is_multiple_of(300) {
            eprintln!(
                "UI_METRICS frames={} render_mean_ms={:.4} interval_mean_ms={:.4} busy={}",
                self.frames,
                self.render_ms / self.frames as f64,
                self.frame_intervals_ms / self.frames as f64,
                self.busy
            );
        }

        view
    }
}

fn control(label: &'static str) -> gpui::Stateful<gpui::Div> {
    gpui::div()
        .id(label)
        .px_3()
        .py_2()
        .rounded_md()
        .bg(rgb(0x344b61))
        .cursor_pointer()
        .child(label)
}

struct Tip;

impl Render for Tip {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        gpui::div()
            .p_3()
            .bg(rgb(0xf0d986))
            .text_color(rgb(0x152231))
            .child("GPUI tooltip over GPU pixels")
    }
}

/// Runs the macOS surface demo with GPUI Fast.
#[rake::app]
fn app() -> rake::App {
    rake::App::new("dev.rake.gpu-surface-spike")
        .window_options(gpui::WindowOptions {
            window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds::new(
                gpui::point(px(60.), px(60.)),
                gpui::size(px(960.), px(720.)),
            ))),
            window_background: gpui::WindowBackgroundAppearance::Transparent,
            ..Default::default()
        })
        .root(|_, window, cx| {
            window.set_window_title("Rake macOS GPU Surface");

            cx.new(|cx| Demo::new(window, cx)).into()
        })
}
