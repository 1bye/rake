//! Browser history is a workspace visit journal, distinct from each tab's history.

use serde::{Deserialize, Serialize};

use super::{EntryId, RouteAddress, RuntimeId, TabId};

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
#[derive(Clone, Copy, Debug, Default)]
pub struct WebConfig {
    pub(crate) routing: WebRouting,
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

    use super::{BrowserCheckpoint, HistoryChange, WebRouting};
    use crate::router::{Navigator, OperationError, Route, RouteDecodeError, Router};

    pub(crate) fn location<R: Route>(routing: WebRouting) -> Result<R, OperationError> {
        let window = web_sys::window().ok_or(OperationError::RuntimeShutdown)?;
        let location = window.location();

        let uri = match routing {
            WebRouting::Path => format!(
                "{}{}",
                location.pathname().unwrap_or_default(),
                location.search().unwrap_or_default()
            ),
            WebRouting::Hash => {
                let hash = location.hash().unwrap_or_default();

                hash.strip_prefix('#')
                    .filter(|value| !value.is_empty())
                    .unwrap_or("/")
                    .to_owned()
            }
        };

        R::from_uri(&uri).map_err(Into::into)
    }

    pub(crate) struct BrowserHistory {
        window: web_sys::Window,
        navigation: Closure<dyn FnMut(web_sys::Event)>,
    }

    impl BrowserHistory {
        pub(crate) fn attach<R: Route>(
            router: &Router<R>,
            routing: WebRouting,
        ) -> Result<Self, OperationError> {
            let window = web_sys::window().ok_or(OperationError::RuntimeShutdown)?;

            let history = window
                .history()
                .map_err(|_| invalid("History API unavailable"))?;

            let last_visit = Rc::new(RefCell::new(None));
            let written_visit = last_visit.clone();
            let written_window = window.clone();

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

                let uri = match routing {
                    WebRouting::Path => uri,
                    WebRouting::Hash => format!("#{uri}"),
                };

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
                    None => match location::<R>(routing) {
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

            if routing == WebRouting::Hash
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
