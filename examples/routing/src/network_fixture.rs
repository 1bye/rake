//! Disposable HTTP inspection commands, compiled only with network-test.

use rake::http::{CancellationToken, HttpClient};
use rake::prelude::*;
use std::time::Duration;

#[rake::data(schema)]
#[derive(rake::Command)]
#[command(id = "test.network", output = Output, capability = Headless)]
#[serde(deny_unknown_fields)]
struct Fixture {
    url: String,
    mode: String,
}

#[rake::data(schema)]
struct Output {
    status: u16,
    bytes: usize,
}

pub(super) fn register(commands: &Commands) {
    commands
        .register::<Fixture, _, _>(|fixture, context| async move {
            let http = context
                .http()
                .map_err(|error| CommandError::Failed(error.to_string()))?;
            execute(fixture, http, context.cancellation).await
        })
        .expect("network fixture registration");
}

async fn execute(
    fixture: Fixture,
    http: HttpClient,
    token: CancellationToken,
) -> Result<Output, CommandError> {
    let request = match fixture.mode.as_str() {
        "post" => http
            .post(&fixture.url)
            .json(&rake::serde_json::json!({"message":"café 你好 🙂","count":3})),
        "get" | "cancel" | "timeout" | "abandon" => http.get(&fixture.url),
        _ => {
            return Err(CommandError::Failed(
                "Use get, post, cancel, timeout or abandon".into(),
            ));
        }
    };
    let request = request
        .header("x-rake-fixture", "network")
        .header("authorization", "Bearer fixture-secret")
        .cancellation(token.clone());
    let request = if fixture.mode == "timeout" {
        request.timeout(Duration::from_millis(50))
    } else {
        request
    };
    let response = request
        .send()
        .await
        .map_err(|error| CommandError::Failed(error.to_string()))?;
    let status = response.status();
    if fixture.mode == "abandon" {
        drop(response);
        return Ok(Output { status, bytes: 0 });
    }
    if fixture.mode == "cancel" {
        token.cancel();
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| CommandError::Failed(error.to_string()))?;
    Ok(Output {
        status,
        bytes: bytes.len(),
    })
}

pub(super) struct Panel {
    base: Option<String>,
    task: Option<TaskHandle>,
    cancellation: CancellationToken,
    result: std::rc::Rc<std::cell::RefCell<String>>,
}

impl Default for Panel {
    fn default() -> Self {
        #[cfg(not(target_family = "wasm"))]
        let base = std::env::var("RAKE_NETWORK_FIXTURE_URL").ok();
        #[cfg(target_family = "wasm")]
        let base = None;
        Self {
            base,
            task: None,
            cancellation: CancellationToken::new(),
            result: std::rc::Rc::new(std::cell::RefCell::new("Choose a request below.".into())),
        }
    }
}

impl Panel {
    fn busy(&self) -> bool {
        self.task
            .as_ref()
            .and_then(|task| task.snapshot().ok())
            .is_some_and(|task| {
                matches!(
                    task.state,
                    rake::tasks::TaskState::Queued
                        | rake::tasks::TaskState::Running
                        | rake::tasks::TaskState::Cancelling
                )
            })
    }

    fn start(&mut self, page: &PageContext<super::Destination>, path: &str, mode: &str) {
        if self.busy() {
            return;
        }
        let Some(base) = &self.base else {
            return;
        };
        self.cancellation = CancellationToken::new();
        let token = self.cancellation.clone();
        let result = self.result.clone();
        let fixture = Fixture {
            url: format!("{base}{path}"),
            mode: mode.into(),
        };
        let http = page.services().http();
        *self.result.borrow_mut() = format!(
            "Sending {} {path}…",
            if mode == "post" { "POST" } else { "GET" }
        );
        match page.tab().tasks().spawn("HTTP request", async move {
            match execute(fixture, http, token).await {
                Ok(output) => {
                    *result.borrow_mut() =
                        format!("HTTP {} · {} bytes received", output.status, output.bytes);
                    Ok(())
                }
                Err(error) => {
                    *result.borrow_mut() = error.to_string();
                    Err(TaskError::Failed(error.to_string()))
                }
            }
        }) {
            Ok(task) => self.task = Some(task),
            Err(error) => *self.result.borrow_mut() = error.to_string(),
        }
    }
}

pub(super) fn controls(
    screen: &super::Screen,
    cx: &mut rake::gpui::Context<super::Screen>,
) -> impl rake::gpui::IntoElement + use<> {
    use rake::gpui::prelude::*;
    use rake::gpui::{div, rgb};
    let panel = &screen.network;
    let busy = panel.busy();
    div()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .items_center()
        .gap_2()
        .child("HTTP requests")
        .child(
            panel
                .base
                .as_ref()
                .map(|base| format!("Server: {base}"))
                .unwrap_or_else(|| "HTTP demo server is unavailable.".into()),
        )
        .child(
            div().flex().flex_wrap().justify_center().gap_2().children(
                [
                    ("network-get", "GET JSON", "/json?q=from-app", "get"),
                    ("network-post", "POST JSON", "/json?q=from-app", "post"),
                    ("network-binary", "GET binary", "/binary", "get"),
                    ("network-error", "HTTP 503", "/error", "get"),
                    ("network-slow", "Slow request", "/slow-ui", "get"),
                    ("network-timeout", "Timeout", "/slow", "timeout"),
                ]
                .into_iter()
                .map(|(id, label, path, mode)| {
                    div()
                        .id(id)
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .bg(rgb(0x29424b))
                        .cursor_pointer()
                        .when(busy || panel.base.is_none(), |view| view.opacity(0.5))
                        .child(label)
                        .on_click(cx.listener(move |screen, _, _, cx| {
                            screen.network.start(&screen.page, path, mode);
                            cx.notify();
                        }))
                        .inspect(id)
                }),
            ),
        )
        .child(
            div()
                .id("network-cancel")
                .px_3()
                .py_2()
                .rounded_md()
                .bg(rgb(0x365849))
                .cursor_pointer()
                .when(!busy, |view| view.opacity(0.5))
                .child("Cancel request")
                .on_click(cx.listener(|screen, _, _, cx| {
                    screen.network.cancellation.cancel();
                    cx.notify();
                }))
                .inspect("network-cancel"),
        )
        .child(screen.network.result.borrow().clone())
}
