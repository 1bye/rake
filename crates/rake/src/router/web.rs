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
    pub(crate) routing: WebRouting,
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

    fn write_uri(&self, uri: &str) -> String {
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

#[cfg(target_family = "wasm")]
pub(crate) mod browser {
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::{JsCast, JsValue, closure::Closure};

    use super::{BrowserCheckpoint, HistoryChange, WebConfig, WebRouting};
    use crate::router::{Navigator, OperationError, Route, RouteDecodeError, Router};

    pub(crate) fn configuration(mut config: WebConfig) -> Result<WebConfig, OperationError> {
        let window = web_sys::window().ok_or(OperationError::RuntimeShutdown)?;
        let document = window.document().ok_or(OperationError::RuntimeShutdown)?;

        let meta = document
            .query_selector("meta[name='rake-web-config']")
            .map_err(|_| invalid("deployment metadata unavailable"))?;

        if let Some(meta) = meta {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Deployment {
                base_path: String,
                routing: String,
            }

            let value: Deployment =
                serde_json::from_str(&meta.get_attribute("content").unwrap_or_default())
                    .map_err(|_| invalid("invalid deployment metadata"))?;

            config = config.base_path(value.base_path)?;
            config.routing = match value.routing.as_str() {
                "path" => WebRouting::Path,
                "hash" => WebRouting::Hash,
                _ => return Err(invalid("invalid deployment routing mode")),
            };
        }

        Ok(config)
    }

    pub(crate) fn location<R: Route>(config: &WebConfig) -> Result<R, OperationError> {
        let window = web_sys::window().ok_or(OperationError::RuntimeShutdown)?;

        let url = window
            .location()
            .href()
            .map_err(|_| invalid("location unavailable"))?;

        config.destination_from_url(&url).map_err(Into::into)
    }

    pub(crate) struct BrowserHistory {
        window: web_sys::Window,
        navigation: Closure<dyn FnMut(web_sys::Event)>,
    }

    impl BrowserHistory {
        pub(crate) fn attach<R: Route>(
            router: &Router<R>,
            config: WebConfig,
        ) -> Result<Self, OperationError> {
            let window = web_sys::window().ok_or(OperationError::RuntimeShutdown)?;

            let history = window
                .history()
                .map_err(|_| invalid("History API unavailable"))?;

            let last_visit = Rc::new(RefCell::new(None));
            let written_visit = last_visit.clone();
            let written_window = window.clone();
            let written_config = config.clone();
            let hash_routing = config.routing == WebRouting::Hash;

            router.on_history(move |change| {
                let (checkpoint, uri, push) = match change {
                    HistoryChange::Push(checkpoint, uri) => (Some(checkpoint), uri, true),
                    HistoryChange::Replace(checkpoint, uri) => (Some(checkpoint), uri, false),
                    HistoryChange::Clear => (None, "/".into(), false),
                };

                let state = checkpoint
                    .map(|checkpoint| serde_json::to_string(&checkpoint).map(JsValue::from))
                    .transpose()
                    .map_err(|error| error.to_string())?
                    .unwrap_or(JsValue::NULL);

                let uri = written_config.write_uri(&uri);

                let result = if push {
                    history.push_state_with_url(&state, "", Some(&uri))
                } else {
                    history.replace_state_with_url(&state, "", Some(&uri))
                };

                result.map_err(|error| format!("browser history update failed: {error:?}"))?;
                *written_visit.borrow_mut() = Some(visit(&written_window));

                Ok(())
            });
            let navigator: Navigator<R> = router.navigator();
            let observed_window = window.clone();

            let navigation = Closure::new(move |_: web_sys::Event| {
                let current = visit(&observed_window);

                if last_visit.borrow().as_ref() == Some(&current) {
                    return;
                }

                *last_visit.borrow_mut() = Some(current.clone());
                let checkpoint = current
                    .1
                    .and_then(|value| serde_json::from_str::<BrowserCheckpoint>(&value).ok());

                match checkpoint {
                    Some(checkpoint) => {
                        drop(navigator.restore(checkpoint));
                    }
                    None => match location::<R>(&config) {
                        Ok(destination) => {
                            drop(navigator.restore_destination(destination));
                        }
                        Err(error) => navigator.reconcile_history(error.to_string()),
                    },
                }
            });

            window
                .add_event_listener_with_callback("popstate", navigation.as_ref().unchecked_ref())
                .map_err(|_| invalid("could not install popstate listener"))?;

            if hash_routing
                && window
                    .add_event_listener_with_callback(
                        "hashchange",
                        navigation.as_ref().unchecked_ref(),
                    )
                    .is_err()
            {
                let _ = window.remove_event_listener_with_callback(
                    "popstate",
                    navigation.as_ref().unchecked_ref(),
                );

                return Err(invalid("could not install hashchange listener"));
            }

            Ok(Self { window, navigation })
        }
    }

    impl Drop for BrowserHistory {
        fn drop(&mut self) {
            for event in ["popstate", "hashchange"] {
                let _ = self.window.remove_event_listener_with_callback(
                    event,
                    self.navigation.as_ref().unchecked_ref(),
                );
            }
        }
    }

    fn visit(window: &web_sys::Window) -> (String, Option<String>) {
        (
            window.location().href().unwrap_or_default(),
            window
                .history()
                .ok()
                .and_then(|history| history.state().ok())
                .and_then(|state| state.as_string()),
        )
    }

    fn invalid(reason: &str) -> OperationError {
        RouteDecodeError::InvalidUri(reason.into()).into()
    }
}
