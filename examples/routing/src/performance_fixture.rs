//! Disposable measured draw/async-work fixture, compiled only with performance-test.

use rake::gpui::prelude::*;
use rake::gpui::{Context, canvas, div, rgb};
use rake::prelude::*;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;
use tracing::Instrument;

static SLOW_DRAWS: AtomicU32 = AtomicU32::new(0);
static PAINTS: AtomicU64 = AtomicU64::new(0);

#[rake::data(schema)]
#[derive(rake::Command)]
#[command(id = "test.performance", output = Output, capability = Headless)]
#[serde(deny_unknown_fields)]
struct Workload {
    slow_draws: u32,
}

#[rake::data(schema)]
struct Output {
    task: String,
    status: u16,
    bytes: usize,
}

pub(super) fn register(commands: &Commands, http: rake::http::HttpClient) {
    commands
        .register::<Workload, _, _>(move |command, context| {
            let http = http.clone();
            async move {
                if command.slow_draws > 3 {
                    return Err(CommandError::Failed("Use at most three slow draws".into()));
                }
                let base = std::env::var("RAKE_NETWORK_FIXTURE_URL").map_err(|_| {
                    CommandError::Failed("Start the local performance fixture server".into())
                })?;
                SLOW_DRAWS.store(command.slow_draws, Ordering::Relaxed);
                let output = Rc::new(Cell::new(None));
                let result = output.clone();
                let task = context
                    .app
                    .tasks()
                    .spawn_with("Performance HTTP work", move |run| {
                        async move {
                            rake::tasks::sleep(Duration::from_millis(30)).await;
                            let response=http.post(&format!("{base}/json?from=performance"))
                .json(&rake::serde_json::json!({"probe":"performance","text":"café 你好 🙂"}))
                .cancellation(run.cancellation()).send().await?;
                            let status = response.status();
                            let bytes = response.bytes().await?;
                            result.set(Some((status, bytes.len())));
                            Ok(())
                        }
                        .instrument(tracing::info_span!("performance.fixture.work"))
                    })
                    .map_err(|error| CommandError::Failed(error.to_string()))?;
                let state = task
                    .wait()
                    .await
                    .map_err(|error| CommandError::Failed(error.to_string()))?;
                let (status, bytes) = output.get().ok_or_else(|| {
                    CommandError::Failed(format!("Performance work ended: {state:?}"))
                })?;
                Ok(Output {
                    task: task.id().to_string(),
                    status,
                    bytes,
                })
            }
        })
        .expect("performance fixture registration");
}

pub(super) fn probe(cx: &mut Context<super::Screen>) -> impl IntoElement + use<> {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap_2()
        .flex_shrink_0()
        .child(
            div()
                .id("performance-work")
                .px_3()
                .py_2()
                .rounded_md()
                .bg(rgb(0x365849))
                .cursor_pointer()
                .child("Run performance workload")
                .on_click(cx.listener(|screen, _, _, cx| {
                    screen.command_sequence += 1;
                    screen.task_error = screen
                        .page
                        .entry()
                        .id()
                        .map_err(|error| error.to_string())
                        .and_then(|entry| {
                            screen
                                .page
                                .dispatch(
                                    Workload { slow_draws: 3 },
                                    format!("performance-{entry}-{}", screen.command_sequence),
                                )
                                .map_err(|error| error.to_string())
                        })
                        .err();
                    cx.notify();
                }))
                .inspect("performance-work"),
        )
        .child(format!(
            "Probe paints: {} · slow draws remaining: {}",
            PAINTS.load(Ordering::Relaxed),
            SLOW_DRAWS.load(Ordering::Relaxed)
        ))
        .child(
            canvas(
                |_, _, _| (),
                |_, (), _, _| {
                    PAINTS.fetch_add(1, Ordering::Relaxed);
                    #[cfg(not(target_family = "wasm"))]
                    if SLOW_DRAWS
                        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                            count.checked_sub(1)
                        })
                        .is_ok()
                    {
                        std::thread::sleep(Duration::from_millis(25));
                    }
                },
            )
            .size_4()
            .inspect("performance.probe"),
        )
}
