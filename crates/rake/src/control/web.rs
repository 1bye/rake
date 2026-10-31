//! Browser-initiated, same-origin development bridge over authenticated HTTP polling.

use super::{HostClient, MAX_MESSAGE_BYTES, Request, RuntimeClient};
use crate::http::HttpClient;
use crate::router::Route;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Development-only connection data, injected into the served HTML by the broker.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebBridgeConfig {
    /// Same-origin broker base URL. Cross-origin URLs are rejected.
    pub url: String,
    /// Random session credential, never placed in the application URL.
    pub token: String,
    /// Immutable published build served to this page (zero outside the supervisor).
    #[serde(default)]
    pub build_id: u64,
}

impl WebBridgeConfig {
    /// Reads the optional `rake-dev-bridge` meta element. Static production builds
    /// contain no element and need no broker or development connection.
    pub fn from_document() -> Result<Option<Self>, String> {
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or("browser document unavailable")?;

        let meta = document
            .query_selector("meta[name='rake-dev-bridge']")
            .map_err(|error| format!("{error:?}"))?;

        let Some(meta) = meta else {
            return Ok(None);
        };

        let config: Self = serde_json::from_str(
            &meta
                .get_attribute("content")
                .ok_or("missing bridge config")?,
        )
        .map_err(|error| error.to_string())?;

        config.validate()?;

        Ok(Some(config))
    }

    fn validate(&self) -> Result<(), String> {
        let origin = web_sys::window()
            .ok_or("browser window unavailable")?
            .location()
            .origin()
            .map_err(|error| format!("{error:?}"))?;

        if self.url != origin || self.token.len() < 32 || self.token.len() > 128 {
            return Err(
                "development bridge requires a same-origin URL and session credential".into(),
            );
        }

        Ok(())
    }
}

#[derive(Deserialize)]
struct Delivery {
    ticket: String,
    #[serde(default)]
    request: Option<Request>,
    #[serde(default)]
    reload_build: Option<u64>,
}

/// Owns a browser's outbound development connection. Dropping `run` stops polling
/// and cancels in-flight Fetch requests; it never cancels submitted operations.
pub struct WebBridge {
    config: WebBridgeConfig,
    http: HttpClient,
}

impl WebBridge {
    /// Uses a dedicated bounded HTTP client, outside application diagnostic collection
    /// so polling does not flood application requests or expose control credentials.
    pub fn new(config: WebBridgeConfig) -> Result<Self, crate::http::Error> {
        config
            .validate()
            .map_err(crate::http::Error::Configuration)?;

        Ok(Self {
            config,
            http: HttpClient::new(crate::http::HttpConfig::new())?,
        })
    }

    async fn post<T: Serialize + ?Sized, U: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        value: &T,
    ) -> Result<U, String> {
        let response = self
            .http
            .post(&format!("{}/__rake/{path}", self.config.url))
            .header("X-Rake-Token", &self.config.token)
            .json(value)
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|error| error.to_string())?;
        let status = response.status();
        let bytes = response.bytes().await.map_err(|error| error.to_string())?;

        if bytes.len() > MAX_MESSAGE_BYTES || !(200..300).contains(&status) {
            return Err(format!(
                "development bridge HTTP {status} or oversized response"
            ));
        }

        serde_json::from_slice(&bytes).map_err(|error| error.to_string())
    }

    pub(super) async fn reload_runtime<R: Route>(
        &self,
        client: &RuntimeClient<R>,
        application_id: &str,
        build: u64,
    ) -> Result<(), String> {
        use crate::reload::{ReloadMechanism, ReloadPhase, ReloadRequest};

        let router = client.router();

        if router.reload_status().phase == ReloadPhase::RestoreFailed {
            router.reload_page()?;
            futures::future::pending::<()>().await;
        }

        let request = ReloadRequest {
            id: format!("web-build-{build}"),
            mechanism: ReloadMechanism::WebPageReload,
        };
        let mut prepared = false;
        let result =
            async {
                // Busy preparation is retried by the broker; keep polling so CLI decisions can settle.
                router
                    .prepare_reload(request.clone())
                    .map_err(|error| error.to_string())?;
                prepared = true;

                for _ in 0..200 {
                    if router.reload_status().phase == ReloadPhase::Ready {
                        break;
                    }

                    crate::http::sleep(Duration::from_millis(50)).await;
                }

                let checkpoint = router
                    .reload_checkpoint(application_id)
                    .map_err(|error| error.to_string())?;

                crate::reload::host::store(&checkpoint)?;
                let _: serde_json::Value = self.post("reload_result", &serde_json::json!({
                "runtime_id": client.info().runtime_id, "build_id": build, "error": null,
                "reload": router.reload_status(),
            })).await?;
                // No await between the final capture and full-page replacement.
                crate::reload::host::store(
                    &router
                        .reload_checkpoint(application_id)
                        .map_err(|error| error.to_string())?,
                )?;
                router.reload_page()?;
                futures::future::pending::<()>().await;

                Ok(())
            }
            .await;

        if result.is_err() && prepared {
            let _ = router.finish_reload(&request.id, false);
        }

        result
    }

    /// Reconnects after transient errors under the same RuntimeId. Duplicate broker
    /// deliveries use the runtime's request keys; the broker cannot add host privileges.
    pub async fn run<R: Route>(&self, client: RuntimeClient<R>, application_id: &str) {
        self.run_host(HostClient::new(client), application_id).await;
    }

    pub(crate) async fn run_host(&self, client: HostClient, application_id: &str) {
        loop {
            let session = async {
                let _: serde_json::Value = self.post("connect", &serde_json::json!({"application_id": application_id, "info": client.info(), "build_id": self.config.build_id, "reload": client.reload_status()})).await?;

                if let Some(document) = web_sys::window().and_then(|window| window.document())
                    && let Some(root) = document.document_element()
                {
                    let _ = root
                        .set_attribute("data-rake-runtime", &client.info().runtime_id.to_string());
                }

                loop {
                    let deliveries: Vec<Delivery> = self
                        .post(
                            "poll",
                            &serde_json::json!({"runtime_id": client.info().runtime_id}),
                        )
                        .await?;

                    for delivery in deliveries {
                        if let Some(build) = delivery.reload_build {
                            let result = client.reload(self, application_id, build).await;
                            let _: serde_json::Value = self
                                .post(
                                    "reload_result",
                                    &serde_json::json!({
                                        "runtime_id": client.info().runtime_id, "build_id": build,
                                        "error": result.err(), "reload": client.reload_status(),
                                    }),
                                )
                                .await?;

                            continue;
                        }

                        let _response = client.begin_response();
                        let result =
                            client.request(delivery.request.ok_or("missing development request")?);
                        let _: serde_json::Value = self.post("reply", &serde_json::json!({"runtime_id": client.info().runtime_id, "ticket": delivery.ticket, "result": result})).await?;
                    }
                }

                #[allow(unreachable_code)]
                Ok::<(), String>(())
            };

            if let Err(error) = session.await {
                crate::warn!(%error, "Development bridge disconnected; reconnecting");
            }

            crate::http::sleep(Duration::from_secs(1)).await;
        }
    }
}
