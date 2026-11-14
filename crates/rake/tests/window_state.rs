//! Covers: public policy/reset API and preservation of unrelated preferences.
//! Real: native preference storage. Doubles: none. Requires: no GPU or display.

#[cfg(not(target_family = "wasm"))]
#[test]
fn reset_loads_existing_records_and_preserves_other_preferences() {
    use rake::{platform::Preferences, window_state::WindowState};

    let directory = tempfile::tempdir().expect("isolated preferences directory");
    let preferences = Preferences::at(directory.path());
    let state = WindowState::new(preferences.clone());

    futures::executor::block_on(async {
        preferences
            .set("theme", "dark")
            .await
            .expect("seed unrelated preference");
        let record = serde_json::json!({
            "bounds": { "x": 50.0, "y": 50.0, "width": 600.0, "height": 400.0 },
            "display": null, "mode": "windowed", "decoration_height": 0.0
        });

        preferences
            .set(
                "rake-window-state-v1",
                &serde_json::json!({
                    "version": 1, "windows": { "main": record, "editor": record }
                })
                .to_string(),
            )
            .await
            .expect("seed saved windows");
        state.reset("main").await.expect("reset existing placement");
        let value = preferences
            .get("rake-window-state-v1")
            .await
            .expect("read saved windows")
            .expect("placement document");

        let document: serde_json::Value =
            serde_json::from_str(&value).expect("valid placement JSON");

        assert!(document["windows"].get("main").is_none());
        assert_eq!(document["windows"]["editor"], record);
        assert_eq!(
            preferences.get("theme").await.expect("read theme"),
            Some("dark".into())
        );
    });
}

#[cfg(feature = "gpui")]
#[test]
fn policy_survives_the_typed_routes_builder_transition() {
    #[derive(Clone, rake::Route)]
    enum Destination {
        #[route(id = "home", path = "/")]
        Home,
    }

    let preferences = rake::platform::Platform::default()
        .preferences("window-state.api-test")
        .expect("valid namespace");

    let state = rake::window_state::WindowState::new(preferences)
        .keys(|id| (id == rake::router::WindowId(1)).then(|| "main".into()));

    let app: rake::App<Destination> = rake::App::new("window-state.api-test")
        .window_state(state)
        .routes(rake::router::__routes(|_: &Destination, _, _, _| {
            Err(rake::router::MountError("unused fixture".into()))
        }));

    assert!(app.services().is_ok());
}
