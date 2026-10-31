//! Real browser regression fixture for Rust worker ownership and guarded reload.

#[cfg(target_family = "wasm")]
mod browser {
    use rake::prelude::*;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use wasm_bindgen::prelude::*;

    thread_local! {
        static CONTEXT: RefCell<Option<RuntimeContext>> = const { RefCell::new(None) };
        static ALLOW: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    }

    #[wasm_bindgen]
    pub fn start() -> Result<(), JsValue> {
        let services = Services::from_config(
            HttpConfig::new(),
            Collector::default(),
            ServiceRegistry::new(),
        )
        .map_err(js_error)?;
        let runtime = Runtime::new(RouterConfig::default(), services);
        let allow = ALLOW.with(Rc::clone);
        runtime
            .quit_guard(move |_| {
                let allowed = allow.get();
                async move {
                    if allowed {
                        Ok(())
                    } else {
                        Err(rake::router::GuardReason("unsaved fixture draft".into()))
                    }
                }
            })
            .map_err(js_error)?;
        CONTEXT.with(|slot| *slot.borrow_mut() = Some(runtime.context()));
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(error) = runtime.run().await {
                rake::error!(%error, "browser fixture stopped");
            }
        });
        Ok(())
    }

    #[wasm_bindgen]
    pub fn allow_quit(allow: bool) {
        ALLOW.with(|value| value.set(allow));
    }

    #[wasm_bindgen]
    pub async fn reload_update() -> Result<String, JsValue> {
        let context = CONTEXT
            .with(|slot| slot.borrow().clone())
            .ok_or_else(|| JsValue::from_str("fixture not started"))?;
        let deployment = rake::web::Deployment::from_document()
            .map_err(js_error)?
            .ok_or_else(|| JsValue::from_str("deployment missing"))?;
        let operation = context
            .reload_web_update(&deployment, QuitOptions::default())
            .await
            .map_err(js_error)?;
        match operation.wait().await.map_err(js_error)? {
            Outcome::Committed(_) => Ok("committed".into()),
            Outcome::Blocked(_) => Ok("blocked".into()),
            Outcome::Cancelled(_) => Ok("cancelled".into()),
        }
    }

    #[wasm_bindgen]
    pub fn cancel_worker() {
        drop(rake::web::run_worker(
            "never.worker.js",
            &rake::serde_json::json!({}),
            std::time::Duration::from_secs(30),
        ));
    }

    fn js_error(error: impl std::fmt::Display) -> JsValue {
        JsValue::from_str(&error.to_string())
    }
}
