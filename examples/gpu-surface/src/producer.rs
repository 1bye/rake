//! A producer thread owns wgpu; input is a bounded shared snapshot, geometry uses a channel.
use super::input_layer::InputLayer;
use rake::{
    gpui::{self, App, FocusHandle, Window},
    overlay::OverlayState,
    surface::{GpuSurface, SurfaceState},
};
use std::rc::Rc;
use std::{
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use surface_spike_renderer::{NativeInputController, Renderer};

pub(super) struct Producer {
    commands: mpsc::Sender<SurfaceState>,
    pub(super) input: Rc<InputLayer>,
    controller: NativeInputController,
}

impl Producer {
    pub(super) fn start(
        preview: &GpuSurface,
        window: &mut Window,
        cx: &mut App,
        fallback: FocusHandle,
        dialog: OverlayState,
        popup: OverlayState,
    ) -> Self {
        let controller = NativeInputController::default();
        let thread_input = controller.clone();
        let input = InputLayer::new(controller.clone(), window, cx, fallback, dialog, popup);
        let (commands, changes) = mpsc::channel::<SurfaceState>();
        let target = preview.metal_layer();
        thread::Builder::new()
            .name("independent-wgpu-28".into())
            .spawn(move || {
                let result = (|| -> anyhow::Result<()> {
                    let mut renderer = pollster::block_on(Renderer::attach(target))?;
                    eprintln!("PRODUCER_LIMITS {:?}", renderer.limits());
                    let mut state = SurfaceState::default();
                    let mut last_input = String::new();
                    loop {
                        let frame_started = Instant::now();
                        match changes.recv_timeout(if state.visible {
                            Duration::ZERO
                        } else {
                            Duration::from_millis(100)
                        }) {
                            Ok(change) => state = change,
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                        }
                        for change in changes.try_iter() {
                            state = change;
                        }
                        if state.closed {
                            break;
                        }
                        renderer.update_input(&thread_input);
                        let received = renderer.input_diagnostics();
                        if received != last_input {
                            eprintln!("NATIVE_INPUT {received}");
                            last_input = received;
                        }
                        if state.visible {
                            renderer.frame(state.width_px, state.height_px)?;
                            if renderer.metrics().frames.is_multiple_of(300) {
                                eprintln!("PRODUCER_METRICS {:?}", renderer.metrics());
                            }
                        }
                        thread::sleep(
                            Duration::from_micros(16_667).saturating_sub(frame_started.elapsed()),
                        );
                    }
                    eprintln!("PRODUCER_STOP {:?}", renderer.metrics());
                    Ok(())
                })();
                if let Err(error) = result {
                    eprintln!("PRODUCER_ERROR {error:#}");
                }
            })
            .expect("start independent renderer thread");
        let sender = commands.clone();
        let next_input = input.clone();
        preview.on_change(move |state| {
            next_input.set_visible(state.visible && !state.closed);
            eprintln!("HOST_STATE {state:?}");
            let _ = sender.send(state);
        });
        Self {
            commands,
            input,
            controller,
        }
    }

    pub(super) fn cancel(&self, window: &mut Window) {
        self.input.cancel(window);
    }

    pub(super) fn input_layer(
        &self,
        enabled: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> gpui::AnyElement {
        self.input.element(enabled, window, cx)
    }

    pub(super) fn diagnostics(&self) -> String {
        self.controller.diagnostics()
    }
}

impl Drop for Producer {
    fn drop(&mut self) {
        self.input.stop();
        let _ = self.commands.send(SurfaceState {
            closed: true,
            ..Default::default()
        });
    }
}
