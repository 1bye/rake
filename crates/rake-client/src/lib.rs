//! Standalone development-control discovery and transport, without the Rake runtime.
//! Blocking exchanges have a finite deadline. Call them outside an async UI executor.

use rake_protocol::control::{
    ControlError, Message, PROTOCOL_VERSION, Query, Reply, Request, RuntimeInfo,
};

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

#[cfg(unix)]
pub mod local;

/// A discovered runtime endpoint. Credentials are kept only in owner-readable files.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Endpoint {
    /// Permission-restricted Unix socket.
    Local {
        /// Socket path.
        path: PathBuf,
    },
    /// Authenticated development broker, addressed by runtime ID.
    Web {
        /// Loopback HTTP broker base URL.
        url: String,
        /// Random development session credential.
        token: String,
    },
}

/// An advertisement written only while a development endpoint is owned by its host.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Instance {
    /// Application name supplied by the runner.
    pub application_id: String,
    /// Native application or broker process ID.
    pub pid: u32,
    /// Version, runtime identity and actual capabilities.
    pub info: RuntimeInfo,
    /// Local or browser-initiated broker transport.
    pub endpoint: Endpoint,
}

/// Inspectable discovery, transport and runtime failures.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Local discovery or socket I/O failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// HTTP configuration or exchange failed.
    #[error(transparent)]
    Http(#[from] rake_http::Error),
    /// An advertisement named an invalid broker URL.
    #[error("discovery advertised an invalid loopback broker URL")]
    InvalidBroker,
    /// This platform cannot use a Unix socket endpoint.
    #[error("local endpoints require Unix")]
    UnsupportedEndpoint,
    /// The runtime rejected the request.
    #[error(transparent)]
    Control(#[from] ControlError),
}

/// Executes one blocking query for the exact advertised runtime.
pub fn query(instance: &Instance, query: Query, timeout: Duration) -> Result<Reply, Error> {
    exchange(
        instance,
        &Request {
            protocol: PROTOCOL_VERSION,
            runtime_id: instance.info.runtime_id,
            key: None,
            message: Message::Query(query),
        },
        timeout,
    )
}

/// Exchanges one request with a finite deadline, preserving runtime rejection errors.
/// Web credentials are sent only to a validated loopback HTTP broker.
pub fn exchange(instance: &Instance, request: &Request, timeout: Duration) -> Result<Reply, Error> {
    let reply = match &instance.endpoint {
        Endpoint::Local { path } => {
            #[cfg(unix)]
            {
                local::exchange(path, request, timeout)?
            }

            #[cfg(not(unix))]
            {
                let _ = path;

                return Err(Error::UnsupportedEndpoint);
            }
        }
        Endpoint::Web { url, token } => {
            let parsed = url.parse::<url::Url>().map_err(|_| Error::InvalidBroker)?;

            if parsed.scheme() != "http"
                || !matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
                || parsed.path() != "/"
                || parsed.query().is_some()
                || parsed.fragment().is_some()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
            {
                return Err(Error::InvalidBroker);
            }

            let endpoint = parsed
                .join("__rake/client")
                .map_err(|_| Error::InvalidBroker)?;

            futures::executor::block_on(async {
                rake_http::HttpClient::new(rake_http::HttpConfig::new())?
                    .post(endpoint.as_str())
                    .header("X-Rake-Token", token)
                    .timeout(timeout)
                    .json(&serde_json::json!({"runtime_id": instance.info.runtime_id, "request": request}))
                    .send_json::<Result<Reply, ControlError>>()
                    .await
            })?
        }
    };

    Ok(reply?)
}
