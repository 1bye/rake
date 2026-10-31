//! Browser workers and production deployment updates, distinct from native installers.

#[cfg(target_family = "wasm")]
mod browser;

use crate::router::{OperationError, WebConfig};
use std::time::Duration;

/// Browser task/deployment failures, including explicit native-host rejection.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// This host does not implement the requested browser operation.
    #[error("{0} is unsupported by this host")]
    Unsupported(&'static str),
    /// Deployment metadata or a worker request/reply is invalid.
    #[error("invalid browser data: {0}")]
    Invalid(String),
    /// A browser or worker operation failed.
    #[error("browser operation failed: {0}")]
    Host(String),
    /// The worker exceeded its explicit time budget and was terminated.
    #[error("browser worker timed out")]
    Timeout,
    /// The application runtime rejected the reload request.
    #[error(transparent)]
    Operation(#[from] OperationError),
}

/// One immutable browser deployment identity, generated from its payload contents.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct DeploymentVersion {
    /// Deployment protocol; currently 1.
    pub protocol: u32,
    /// Application identity, checked before offering a deployment update.
    pub application_id: String,
    /// Human-readable Cargo package version.
    pub version: String,
    /// Content identity, allowing two builds of the same version to differ.
    pub build: String,
    /// Deployment root with leading/trailing slashes, independent of asset location.
    pub base_path: String,
}

impl DeploymentVersion {
    /// Validates a generated manifest without granting it navigation authority.
    ///
    /// # Errors
    /// Rejects unknown protocol, malformed paths, empty identity or invalid build IDs.
    pub fn validate(&self) -> Result<(), Error> {
        if self.protocol != 1
            || self.application_id.is_empty()
            || self.application_id.len() > 256
            || self.version.is_empty()
            || self.version.len() > 128
            || self.build.len() != 64
            || !self
                .build
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(Error::Invalid("invalid deployment identity".into()));
        }
        WebConfig::new()
            .base_path(&self.base_path)
            .map_err(|error| Error::Invalid(error.to_string()))?;
        Ok(())
    }
}

/// The running page's immutable deployment identity. Checks only offer updates for
/// this same application/root; URLs are resolved on the current origin.
#[derive(Clone, Debug)]
pub struct Deployment {
    current: DeploymentVersion,
}

impl Deployment {
    /// Captures generated page metadata. Development/unbundled pages return `None`;
    /// native hosts also return `None`. This performs no network operation.
    pub fn from_document() -> Result<Option<Self>, Error> {
        #[cfg(target_family = "wasm")]
        return browser::deployment();
        #[cfg(not(target_family = "wasm"))]
        Ok(None)
    }

    /// Captures a validated deployment identity. Useful for custom browser hosts.
    pub fn new(current: DeploymentVersion) -> Result<Self, Error> {
        current.validate()?;
        Ok(Self { current })
    }

    /// The version currently running, not the latest server version.
    pub fn current(&self) -> &DeploymentVersion {
        &self.current
    }

    /// Validates a same-origin server manifest and returns a changed deployment.
    /// A content rollback is also an update; no ordering is inferred from version text.
    pub fn available(
        &self,
        candidate: DeploymentVersion,
    ) -> Result<Option<DeploymentVersion>, Error> {
        candidate.validate()?;
        if candidate.application_id != self.current.application_id
            || candidate.base_path != self.current.base_path
        {
            return Err(Error::Invalid(
                "deployment belongs to another application or root".into(),
            ));
        }
        Ok((candidate.build != self.current.build).then_some(candidate))
    }

    /// Checks the same-origin version endpoint without caches, redirects or unbounded
    /// response bodies. A check does not activate a service worker or reload a page.
    pub async fn check(&self) -> Result<Option<DeploymentVersion>, Error> {
        #[cfg(target_family = "wasm")]
        return self.available(browser::check(&self.current.base_path).await?);
        #[cfg(not(target_family = "wasm"))]
        Err(Error::Unsupported("browser deployment checks"))
    }
}

/// Starts a dedicated same-origin module worker using a bounded JSON message/reply.
/// The worker receives one JSON string and must reply with one JSON string. Request
/// and reply are limited to 1 MiB. Timeouts, errors and dropping the future terminate
/// the worker; no worker survives its owner. This is explicit CPU parallelism, not
/// the ordinary UI-loop loader executor. It requires no SharedArrayBuffer/COOP/COEP.
///
/// # Errors
/// Rejects native hosts, nonpositive timeouts, oversized JSON and unavailable workers.
pub fn run_worker(
    module: &str,
    input: &serde_json::Value,
    timeout: Duration,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<serde_json::Value, Error>>>> {
    #[cfg(target_family = "wasm")]
    return browser::worker(module, input, timeout);
    #[cfg(not(target_family = "wasm"))]
    {
        let _ = (module, input, timeout);
        Box::pin(async { Err(Error::Unsupported("browser workers")) })
    }
}
