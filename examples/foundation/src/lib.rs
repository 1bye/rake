//! One ordinary application with only the Rake facade as a dependency.

use rake::gpui::prelude::*;
use rake::gpui::{Context, Task, Window, div, rgb};
use rake::prelude::*;

#[rake::data]
struct Message {
    title: String,
}

struct Foundation {
    status: String,
    services: Services,
    request: Option<Task<()>>,
}

#[rake::instrument(skip_all)]
async fn load_message(http: HttpClient) -> rake::http::Result<Message> {
    http.get("message.json").send_json().await
}

impl Render for Foundation {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .justify_center()
            .items_center()
            .gap_4()
            .bg(rgb(0x18232b))
            .text_color(rgb(0xf0f4e8))
            .font_family("IBM Plex Sans")
            .child(div().text_3xl().child("Rake"))
            .child(self.status.clone())
            .child(
                div()
                    .id("load-message")
                    .px_4()
                    .py_2()
                    .rounded_md()
                    .bg(rgb(0x365849))
                    .cursor_pointer()
                    .child("Load JSON")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let http = this.services.http();

                        this.status = "Loading…".into();
                        this.request = Some(cx.spawn(async move |view, cx| {
                            let result = load_message(http).await;

                            let _ = view.update(cx, |view, cx| {
                                view.status = match result {
                                    Ok(message) => {
                                        rake::info!(title = %message.title, "Message loaded");

                                        message.title
                                    }
                                    Err(error) => {
                                        rake::warn!(%error, "Message request failed");

                                        error.to_string()
                                    }
                                };
                                cx.notify();
                            });
                        }));
                        cx.notify();
                    })),
            )
    }
}

/// The same factory configures the native binary and browser Wasm export.
pub fn application() -> App {
    App::new("com.example.rake.foundation")
        .http(HttpConfig::new().base_url("http://127.0.0.1:8080/"))
        .root(|services, _, cx| {
            cx.new(|_| Foundation {
                status: "Portable application foundation".into(),
                services,
                request: None,
            })
            .into()
        })
}

#[rake::app]
fn app() -> App {
    application()
}
