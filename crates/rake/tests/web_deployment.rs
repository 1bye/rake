//! Covers: deployment-prefixed path/hash URLs and typed deep-link round trips.
//! Real: public Web configuration and generated route codecs. Doubles: none.
//! Requires: no browser, GPU, filesystem or network.

use rake::router::{WebConfig, WebRouting};

#[derive(Clone, rake::Route, Debug, PartialEq, Eq)]
enum Destination {
    #[route(id = "home", path = "/")]
    Home,
    #[route(id = "document", path = "/documents/{id}")]
    Document(String),
}

#[test]
fn path_and_hash_deployments_preserve_the_prefix_and_typed_destination() {
    for routing in [WebRouting::Path, WebRouting::Hash] {
        let config = WebConfig::new()
            .routing(routing)
            .base_path("/notes/v1/")
            .unwrap();

        let destination = Destination::Document("A B/漢字".into());
        let uri = config.uri_for(&destination).unwrap();

        assert!(uri.starts_with("/notes/v1/"));
        assert_eq!(
            config
                .destination_from_url::<Destination>(&format!("https://example.test{uri}"))
                .unwrap(),
            destination
        );
        assert!(
            config
                .destination_from_url::<Destination>("https://example.test/other/#/")
                .is_err()
        );
        assert!(
            config
                .destination_from_url::<Destination>("https://example.test/notes/v10/#/")
                .is_err()
        );
        assert!(
            config
                .destination_from_url::<Destination>("file:///notes/v1/")
                .is_err()
        );
    }
}

#[test]
fn deployment_roots_and_invalid_prefixes_have_explicit_results() {
    let config = WebConfig::new().base_path("/notes/").unwrap();

    assert_eq!(config.uri_for(&Destination::Home).unwrap(), "/notes/");
    assert_eq!(
        config
            .destination_from_url::<Destination>("https://example.test/notes")
            .unwrap(),
        Destination::Home
    );
    assert_eq!(
        config
            .destination_from_url::<Destination>("https://example.test/notes/")
            .unwrap(),
        Destination::Home
    );

    for invalid in [
        "", "notes/", "/notes", "//notes/", "/a/../", "/a/./", "/a//b/", "/a%2fb/", "/a?b/",
        "/a#b/", "/漢/",
    ] {
        assert!(
            WebConfig::new().base_path(invalid).is_err(),
            "accepted {invalid}"
        );
    }
}
