//! Covers: static route precedence over a dynamic route with the same segment count.
//! Real: the public route derive and URI decoder.
//! Doubles: none.
//! Requires: no GPU or external resources.

use rake::Route;

#[derive(Clone, Debug, PartialEq, Eq, rake::Route)]
enum Destination {
    #[route(id = "new", path = "/files/new")]
    New,
    #[route(id = "file", path = "/files/{id}")]
    File(String),
}

#[test]
fn static_paths_still_precede_dynamic_paths() {
    assert_eq!(Destination::from_uri("/files/new"), Ok(Destination::New));
    assert_eq!(
        Destination::from_uri("/files/readme"),
        Ok(Destination::File("readme".into()))
    );
}
