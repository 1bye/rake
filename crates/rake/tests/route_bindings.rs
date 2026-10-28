//! Covers: route fields that collide with generated encoder bindings.
//! Real: derive expansion, public route codecs and URI round trips.
//! Doubles: none. Requires: no GPU, browser or external services.

use rake::router::Route;

#[derive(Clone, Debug, PartialEq, Eq, rake::Route)]
enum Destination {
    #[route(id = "required", path = "/required/{params}")]
    Required {
        params: String,
        parameter_0: String,
        __rake_parameter_0: String,
    },
    #[route(id = "optional", path = "/optional")]
    Optional { params: Option<String> },
}

#[test]
fn generated_bindings_do_not_shadow_named_route_fields() {
    for destination in [
        Destination::Required {
            params: "path/value".into(),
            parameter_0: "query value".into(),
            __rake_parameter_0: "another value".into(),
        },
        Destination::Optional {
            params: Some("query/value".into()),
        },
        Destination::Optional { params: None },
    ] {
        let address = destination.encode();

        assert_eq!(Destination::decode(&address).expect("address"), destination);
        assert_eq!(
            Destination::from_uri(&destination.to_uri().expect("URI")).expect("decode"),
            destination
        );
    }
}
