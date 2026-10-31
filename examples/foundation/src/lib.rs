//! One route-free application with automatically bound asynchronous loading.

use rake::gpui::{Context, Window, prelude::*};
use rake::prelude::*;

#[rake::data]
struct Message {
    title: String,
}

struct Foundation {
    message: Loader<Message, rake::http::Error>,
    status: String,
    runtime: RuntimeContext,
}

#[rake::instrument(skip_all)]
async fn load_message(load: LoadContext) -> rake::http::Result<Message> {
    load.http()
        .get("message.json")
        .cancellation(load.cancellation())
        .send_json()
        .await
}

impl Render for Foundation {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let loading = self.message.is_loading(cx);
        let message = match self.message.state(cx) {
            LoadStatus::Idle | LoadStatus::Loading(_) => "Loading…".into(),
            LoadStatus::Ready(message) => message.title.clone(),
            LoadStatus::Failed(error) => error.to_string(),
        };

        rake::view! {
            <main class="size-full p-6 gap-4 bg-[#18232b] text-[#f0f4e8]" tab-navigation>
                <h1>"Rake"</h1>
                <p>{message}</p>
                <button id="load-message" disabled={loading}
                    on:click={cx.listener(|this, _, _, cx| this.message.retry(cx))}>
                    "Reload / Retry"
                </button>
                <button id="managed-job" on:click={cx.listener(|this, _, _, cx| {
                    let work = this.runtime.tasks().spawn("example-job", async {
                        rake::tasks::sleep(std::time::Duration::from_secs(1)).await;
                        rake::info!("Managed example job finished");
                        Ok(())
                    });
                    this.status = match work {
                        Ok(_) => "Job started; normal quit waits for it to finish".into(),
                        Err(error) => error.to_string(),
                    };
                    cx.notify();
                })}>"Run managed job"</button>
                <p>{self.status.clone()}</p>
            </main>
        }
    }
}

/// The same factory configures the native binary and browser Wasm export.
pub fn application() -> App {
    App::new("com.example.rake.foundation")
        .http(HttpConfig::new().base_url("http://127.0.0.1:8080/"))
        .root_with(|runtime, _, cx| {
            cx.new(|cx| Foundation {
                message: runtime.load(cx, load_message),
                status: "Portable application foundation".into(),
                runtime,
            })
            .into()
        })
}

#[rake::app]
fn app() -> App {
    application()
}
