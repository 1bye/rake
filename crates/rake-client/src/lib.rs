//! Standalone development-control discovery and transport, without the Rake runtime.
//! Blocking exchanges are native-only and have a finite total deadline.
//! Call them outside an async UI executor; endpoint metadata remains portable.

use rake_protocol::control::{ControlError, RuntimeInfo};
#[cfg(not(target_family = "wasm"))]
use rake_protocol::control::{MAX_MESSAGE_BYTES, Message, PROTOCOL_VERSION, Query, Reply, Request};

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[cfg(not(target_family = "wasm"))]
use std::time::Duration;

#[cfg(not(target_family = "wasm"))]
thread_local! {
    static DEADLINE: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
}

/// Limits every blocking exchange in a synchronous operation to one total deadline.
/// Nested scopes can shorten the deadline. The previous scope is restored on unwind.
/// This does not cancel a request already delivered or automatically retry mutations.
/// Do not return an unpolled future: the scope applies only while `operation` runs.
#[cfg(not(target_family = "wasm"))]
pub fn with_deadline<T>(deadline: std::time::Instant, operation: impl FnOnce() -> T) -> T {
    struct Restore(Option<std::time::Instant>);
    impl Drop for Restore {
        fn drop(&mut self) {
            DEADLINE.set(self.0);
        }
    }
    let previous = DEADLINE.get();
    let _restore = Restore(previous);
    DEADLINE.set(Some(previous.map_or(deadline, |old| old.min(deadline))));
    operation()
}

#[cfg(not(target_family = "wasm"))]
fn remaining(timeout: Duration) -> std::io::Result<Duration> {
    if timeout.is_zero() || std::time::Instant::now().checked_add(timeout).is_none() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid control timeout",
        ));
    }
    let timeout = DEADLINE.get().map_or(timeout, |deadline| {
        timeout.min(deadline.saturating_duration_since(std::time::Instant::now()))
    });
    if timeout.is_zero() {
        Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "control operation deadline expired",
        ))
    } else {
        Ok(timeout)
    }
}

#[cfg(unix)]
pub mod local;

/// A discovered runtime endpoint. Credentials are kept only in owner-readable files.
#[derive(Clone, Serialize, Deserialize)]
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

impl std::fmt::Debug for Endpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local { path } => formatter.debug_struct("Local").field("path", path).finish(),
            Self::Web { url, .. } => formatter
                .debug_struct("Web")
                .field("url", url)
                .field("token", &"[REDACTED]")
                .finish(),
        }
    }
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
    /// Discovery, socket I/O or control-frame validation failed.
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
///
/// # Errors
/// Returns the same transport, framing and runtime failures as [`exchange`].
#[cfg(not(target_family = "wasm"))]
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
/// Web credentials are sent only to a validated loopback HTTP broker; redirects
/// are rejected. Responses are limited to the protocol's 1 MiB frame budget.
///
/// # Errors
/// Returns [`Error::InvalidBroker`] for invalid broker URLs, [`Error::Io`] for
/// socket/framing failures or oversized replies, [`Error::Http`] for HTTP failures
/// (including timeouts and redirects), and [`Error::Control`] for runtime rejection.
#[cfg(not(target_family = "wasm"))]
pub fn exchange(instance: &Instance, request: &Request, timeout: Duration) -> Result<Reply, Error> {
    let timeout = remaining(timeout)?;
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
                let mut response = rake_http::HttpClient::new(rake_http::HttpConfig::new())?
                    .without_redirects()?
                    .post(endpoint.as_str())
                    .header("X-Rake-Token", token)
                    .timeout(timeout)
                    .json(&serde_json::json!({"runtime_id": instance.info.runtime_id, "request": request}))
                    .send().await?.error_for_status()?;

                let mut bytes = Vec::new();

                while let Some(chunk) = response.next_chunk().await? {
                    if chunk.len() > MAX_MESSAGE_BYTES - bytes.len() {
                        return Err(Error::Io(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "control response too large",
                        )));
                    }

                    bytes.extend_from_slice(&chunk);
                }

                serde_json::from_slice::<Result<Reply, ControlError>>(&bytes)
                    .map_err(rake_http::Error::Decode)
                    .map_err(Error::Http)
            })?
        }
    };

    Ok(reply?)
}
