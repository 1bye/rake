//! Browser history is a workspace visit journal, distinct from each tab's history.

use serde::{Deserialize, Serialize};

use super::{EntryId, Route, RouteAddress, RouteDecodeError, RuntimeId, TabId};

/// URL representation used by the browser host.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WebRouting {
    /// Absolute paths; the deployment server must serve the app shell for deep links.
    #[default]
    Path,
    /// Application URI in the fragment, for hosts without document-route rewrites.
    Hash,
}

/// Browser routing configuration. Native hosts retain it without changing OS behavior.
#[derive(Clone, Debug)]
pub struct WebConfig {
    routing: WebRouting,
    base_path: String,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            routing: WebRouting::Path,
            base_path: "/".into(),
        }
    }
}

impl WebConfig {
    /// Uses path routing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Selects path or hash routing.
    pub fn routing(mut self, routing: WebRouting) -> Self {
        self.routing = routing;

        self
    }

    /// Sets the deployment prefix. It must start/end with `/` and contain only
    /// ASCII letters, digits, `-`, `_`, `.`, `~` and nonempty path segments.
    /// Dot segments, encoded separators and URL query/fragment syntax are rejected.
    pub fn base_path(mut self, path: impl Into<String>) -> Result<Self, RouteDecodeError> {
        let path = path.into();

        if !path.starts_with('/')
            || !path.ends_with('/')
            || !path
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte))
            || (path != "/"
                && path[1..path.len() - 1]
                    .split('/')
                    .any(|part| matches!(part, "" | "." | "..")))
        {
            return Err(RouteDecodeError::InvalidUri(
                "invalid Web deployment base path".into(),
            ));
        }

        self.base_path = path;

        Ok(self)
    }

    /// Produces a browser path or fragment URL, including this deployment's prefix.
    pub fn uri_for<R: Route>(&self, destination: &R) -> Result<String, RouteDecodeError> {
        Ok(self.write_uri(&destination.to_uri()?))
    }

    /// Decodes an absolute HTTP(S) URL within this deployment. Path and hash modes
    /// use the same typed route codec; paths outside the prefix are rejected.
    pub fn destination_from_url<R: Route>(&self, url: &str) -> Result<R, RouteDecodeError> {
        let url = url::Url::parse(url)
            .map_err(|error| RouteDecodeError::InvalidUri(error.to_string()))?;

        if !matches!(url.scheme(), "http" | "https") {
            return Err(RouteDecodeError::InvalidUri(
                "Web routes require HTTP(S)".into(),
            ));
        }

        let path = if url.path() == self.base_path.trim_end_matches('/') {
            ""
        } else {
            url.path().strip_prefix(&self.base_path).ok_or_else(|| {
                RouteDecodeError::InvalidUri("URL is outside the Web deployment".into())
            })?
        };

        let uri = match self.routing {
            WebRouting::Path => format!(
                "/{path}{}",
                url.query()
                    .map(|query| format!("?{query}"))
                    .unwrap_or_default()
            ),
            WebRouting::Hash => url
                .fragment()
                .filter(|fragment| !fragment.is_empty())
                .unwrap_or("/")
                .into(),
        };

        R::from_uri(&uri)
    }

    #[doc(hidden)]
    pub fn routing_mode(&self) -> WebRouting {
        self.routing
    }

    #[doc(hidden)]
    pub fn write_uri(&self, uri: &str) -> String {
        match self.routing {
            WebRouting::Path => format!("{}{}", self.base_path, uri.trim_start_matches('/')),
            WebRouting::Hash => format!("{}#{uri}", self.base_path),
        }
    }
}

/// A reconstructible browser visit with optional live-instance hints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserCheckpoint {
    /// Current schema version, initially 1.
    pub version: u32,
    /// Runtime in which these hints were created.
    pub runtime_id: RuntimeId,
    /// Internal application tab hint.
    pub tab_id: TabId,
    /// Retained screen hint.
    pub entry_id: EntryId,
    /// Complete canonical destination, authoritative when hints are stale.
    pub destination: RouteAddress,
}

/// A committed workspace's intended URL update. Restore never emits Push.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HistoryChange {
    /// Add a visible workspace visit.
    Push(BrowserCheckpoint, String),
    /// Initialize, reconcile or replace the current visit without adding one.
    Replace(BrowserCheckpoint, String),
    /// The last tab closed; clear Rake state and show the root URL.
    Clear,
}
