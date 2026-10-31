//! Wasm bindings for owned module workers and deployment metadata.

use super::{Deployment, DeploymentVersion, Error};
use std::time::Duration;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen(module = "/src/web/browser.js")]
extern "C" {
    #[wasm_bindgen(catch)]
    fn start_worker(module: &str, input: &str, timeout: f64) -> Result<JsValue, JsValue>;
    fn worker_promise(job: &JsValue) -> js_sys::Promise;
    fn cancel_worker(job: &JsValue);
    fn deployment_metadata() -> Option<String>;
    fn check_deployment(base: &str) -> js_sys::Promise;
}

fn error(value: JsValue) -> Error {
    let property = |name| {
        js_sys::Reflect::get(&value, &JsValue::from_str(name))
            .ok()
            .and_then(|value| value.as_string())
    };
    match property("name").as_deref() {
        Some("TimeoutError") => Error::Timeout,
        Some("NotSupportedError") => Error::Unsupported("browser workers"),
        _ => Error::Host(property("message").unwrap_or_else(|| format!("{value:?}"))),
    }
}

struct WorkerOwner(JsValue);
impl Drop for WorkerOwner {
    fn drop(&mut self) {
        cancel_worker(&self.0);
    }
}

pub(super) fn worker(
    module: &str,
    input: &serde_json::Value,
    timeout: Duration,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<serde_json::Value, Error>>>> {
    let raw = input.to_string();
    let result = start_worker(module, &raw, timeout.as_secs_f64() * 1000.).map_err(error);
    // Attach promise observation now, even when a caller drops the unpolled request.
    let initiated = result.map(|job| {
        let future = JsFuture::from(worker_promise(&job));
        (WorkerOwner(job), future)
    });
    Box::pin(async move {
        let (_owner, future) = initiated?;
        let value = future.await.map_err(error)?;
        let raw = value
            .as_string()
            .ok_or_else(|| Error::Invalid("worker reply is not a string".into()))?;
        serde_json::from_str(&raw).map_err(|error| Error::Invalid(error.to_string()))
    })
}

pub(super) fn deployment() -> Result<Option<Deployment>, Error> {
    deployment_metadata()
        .map(|raw| {
            let version =
                serde_json::from_str(&raw).map_err(|error| Error::Invalid(error.to_string()))?;
            Deployment::new(version)
        })
        .transpose()
}

pub(super) async fn check(base: &str) -> Result<DeploymentVersion, Error> {
    let value = JsFuture::from(check_deployment(base))
        .await
        .map_err(error)?;
    serde_json::from_str(
        &value
            .as_string()
            .ok_or_else(|| Error::Invalid("deployment reply is not a string".into()))?,
    )
    .map_err(|error| Error::Invalid(error.to_string()))
}
