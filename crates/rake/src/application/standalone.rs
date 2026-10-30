//! A window-owned inspection host; dropping the root cancels its transport and leases.

use gpui::{AnyView, AppContext, Context, IntoElement, Render, Window};

use crate::{
    Services,
    control::{ControlConfig, standalone::InspectionClient},
    router::WindowId,
};

pub(super) fn mount(
    content: AnyView,
    services: &Services,
    control: ControlConfig,
    application_id: String,
    cx: &mut gpui::App,
) -> AnyView {
    if !control.inspection_enabled() {
        return content;
    }
    let client = InspectionClient::new(services.diagnostics());
    if let Ok(storage) = services.get::<crate::storage::devtools::Storage>() {
        client.set_storage(storage.as_ref().clone());
    }
    cx.new(|cx| InspectionRoot::new(content, client, control, application_id, cx))
        .into()
}

impl InspectionRoot {
    fn new(
        content: AnyView,
        client: InspectionClient,
        control: ControlConfig,
        application_id: String,
        cx: &mut Context<Self>,
    ) -> Self {
        let inspection = client.inspection();
        let host = client.host();
        let drive = async move |view: gpui::WeakEntity<InspectionRoot>, cx: &mut gpui::AsyncApp| {
            let observe = async {
                loop {
                    inspection.listen().await;
                    if view.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            };
            let control = crate::control::run_host_client(control, host, application_id);
            futures::future::select(Box::pin(observe), Box::pin(control)).await;
        };
        #[cfg(not(target_family = "wasm"))]
        let driver = Driver {
            _task: cx.spawn(drive),
        };
        #[cfg(target_family = "wasm")]
        let driver = {
            let view = cx.weak_entity();
            let mut context = cx.to_async();
            let (abort, registration) = futures::future::AbortHandle::new_pair();
            // Transport I/O must wake on microtasks even when the page's frame timers are throttled.
            wasm_bindgen_futures::spawn_local(async move {
                let _ =
                    futures::future::Abortable::new(drive(view, &mut context), registration).await;
            });
            Driver(abort)
        };
        Self {
            content,
            client,
            _driver: driver,
        }
    }
}

#[cfg(not(target_family = "wasm"))]
struct Driver {
    _task: gpui::Task<()>,
}
#[cfg(target_family = "wasm")]
struct Driver(futures::future::AbortHandle);

#[cfg(target_family = "wasm")]
impl Drop for Driver {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct InspectionRoot {
    content: AnyView,
    client: InspectionClient,
    _driver: Driver,
}

impl Drop for InspectionRoot {
    fn drop(&mut self) {
        self.client.close();
    }
}

impl Render for InspectionRoot {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        crate::inspection::inspect(self.content.clone())
            .semantic_id("rake.root")
            .root(self.client.inspection(), 0, WindowId(1))
    }
}

#[cfg(all(test, feature = "view-test-support"))]
mod tests;
