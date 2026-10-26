//! Read-only, bounded HTTP client shared by native and Wasm inspector frontends.

use rake::control::{
    ControlError, MAX_MESSAGE_BYTES, Message, PROTOCOL_VERSION, Query, Reply, Request, RuntimeInfo,
};

use rake::http::{CancellationToken, HttpClient, HttpConfig};
use rake::router::RuntimeId;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::time::Duration;
use url::Url;

/// Private launch configuration. Debug output deliberately excludes its credential.
#[derive(Clone)]
pub struct ConnectionConfig {
    origin: String,
    token: String,
    /// Initial exact runtime ID or application ID; ambiguous application IDs need explicit selection.
    pub instance: Option<String>,
}

impl ConnectionConfig {
    /// Parses a private broker launch URL. Only loopback HTTP and a bounded session
    /// credential are accepted; credentials never become request query parameters.
    pub fn from_launch_url(value: &str) -> Result<Self, String> {
        let url = Url::parse(value).map_err(|_| "Invalid inspector launch URL.")?;

        if url.scheme() != "http"
            || !matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || !matches!(
                url.path(),
                "/" | "/__rake/inspector" | "/__rake/elements" | "/__rake/elements/"
            )
        {
            return Err("Inspector requires a local HTTP broker launch URL.".into());
        }

        let token = url
            .fragment()
            .and_then(|value| value.strip_prefix("__rake_control="))
            .ok_or("Missing inspector session credential.")?;

        if !(32..=128).contains(&token.len())
            || !token
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err("Invalid inspector session credential.".into());
        }

        Ok(Self {
            origin: url.origin().ascii_serialization(),
            token: token.into(),
            instance: None,
        })
    }

    /// Broker origin, safe to show without disclosing its session credential.
    pub fn origin(&self) -> &str {
        &self.origin
    }

    #[cfg(feature = "gui")]
    pub(crate) fn from_host() -> Result<Self, String> {
        #[cfg(not(target_family = "wasm"))]
        {
            let value = std::env::var("RAKE_INSPECTOR_URL").map_err(|_| "Launch with rake inspect --elements, or set RAKE_INSPECTOR_URL to a private inspector launch URL.")?;
            let mut config = Self::from_launch_url(&value)?;

            config.instance = std::env::var("RAKE_INSPECTOR_INSTANCE").ok();

            Ok(config)
        }

        #[cfg(target_family = "wasm")]
        {
            let window = web_sys::window().ok_or("Browser unavailable.")?;

            let storage = window
                .session_storage()
                .map_err(|_| "Session storage unavailable.")?
                .ok_or("Session storage unavailable.")?;

            let token = storage
                .get_item("rake-dev-control-token")
                .map_err(|_| "Cannot read inspector session.")?
                .ok_or("Open the private Elements launch URL.")?;

            let origin = window
                .location()
                .origin()
                .map_err(|_| "Browser origin unavailable.")?;

            let mut config =
                Self::from_launch_url(&format!("{origin}/__rake/elements#__rake_control={token}"))?;

            config.instance = storage
                .get_item("rake-inspector-instance")
                .map_err(|_| "Cannot read runtime selector.")?;

            Ok(config)
        }
    }
}

/// Runtime advertisement without endpoint paths or broker credentials.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Instance {
    /// Application's stable identity.
    pub application_id: String,
    /// Shared runtime identity and capabilities.
    pub info: RuntimeInfo,
    /// Proven reload predecessor, if this runtime restored its checkpoint.
    #[serde(default)]
    pub restored_from: Option<RuntimeId>,
}

/// A dedicated client outside the inspected application's HTTP diagnostics.
#[derive(Clone)]
pub struct Connection {
    config: ConnectionConfig,
    http: HttpClient,
}

impl Connection {
    /// Creates a cancellable local client; it only sends inspection queries.
    pub fn new(config: ConnectionConfig) -> Result<Self, String> {
        Ok(Self {
            config,
            http: HttpClient::new(HttpConfig::new().timeout(Duration::from_secs(5)))
                .map_err(|error| error.to_string())?,
        })
    }

    /// Enumerates the broker's bounded runtime advertisements.
    pub async fn instances(
        &self,
        cancellation: CancellationToken,
    ) -> Result<Vec<Instance>, String> {
        let instances: Vec<Instance> = self
            .post("inspect_instances", &serde_json::json!({}), cancellation)
            .await?;

        if instances.len() > 128 {
            return Err("Runtime advertisement limit exceeded.".into());
        }

        Ok(instances)
    }

    /// Submits a shared owned query to an exact runtime. It never submits mutations.
    pub async fn query(
        &self,
        runtime: RuntimeId,
        query: Query,
        cancellation: CancellationToken,
    ) -> Result<Reply, String> {
        if !matches!(
            query,
            Query::InspectTree {} | Query::InspectElement { .. } | Query::Info {}
        ) {
            return Err("This Inspector only supports Elements queries.".into());
        }

        let request = Request {
            protocol: PROTOCOL_VERSION,
            runtime_id: runtime,
            key: None,
            message: Message::Query(query),
        };

        let reply: Result<Reply, ControlError> = self
            .post(
                "inspect",
                &serde_json::json!({"request": request}),
                cancellation,
            )
            .await?;

        reply.map_err(|error| error.to_string())
    }

    async fn post<T: Serialize, U: DeserializeOwned>(
        &self,
        action: &str,
        body: &T,
        cancellation: CancellationToken,
    ) -> Result<U, String> {
        let mut response = self
            .http
            .post(&format!("{}/__rake/{action}", self.config.origin))
            .header("X-Rake-Token", &self.config.token)
            .json(body)
            .cancellation(cancellation)
            .send()
            .await
            .map_err(|_| "Inspector broker connection failed.".to_string())?;

        if !(200..300).contains(&response.status()) {
            return Err(format!("Inspector broker HTTP {}.", response.status()));
        }

        let mut bytes = Vec::new();

        while let Some(chunk) = response
            .next_chunk()
            .await
            .map_err(|_| "Inspector response interrupted.")?
        {
            if bytes.len() + chunk.len() > MAX_MESSAGE_BYTES {
                return Err("Inspector response exceeds the frame limit.".into());
            }

            bytes.extend(chunk);
        }

        serde_json::from_slice(&bytes).map_err(|_| "Inspector returned an invalid response.".into())
    }
}
