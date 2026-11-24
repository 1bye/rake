pub(crate) mod browser {
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::{JsCast, JsValue, closure::Closure};

    use crate::router::{BrowserCheckpoint, HistoryChange, WebConfig, WebRouting};
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
            config = config.routing(match value.routing.as_str() {
                "path" => WebRouting::Path,
                "hash" => WebRouting::Hash,
                _ => return Err(invalid("invalid deployment routing mode")),
            });
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
            let hash_routing = config.routing_mode() == WebRouting::Hash;

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
