//! Covers: workspace selection and facade-consumer configuration errors.
//! Real: Cargo metadata decoding and portable planning.
//! Doubles: a Cargo metadata response.
//! Requires: no Cargo invocation, native tools, network or credentials.
use rake_dist::{Error, Platform, plan_metadata};
use serde_json::json;

#[test]
fn explicit_workspace_selection_excludes_dependency_packages() {
    let input = serde_json::to_vec(&json!({"workspace_members":["app"],"packages":[{"id":"app","name":"notes","version":"1.2.3","metadata":{"rake":{"bundle":{"schema_version":1}}}},{"id":"dep","name":"library","version":"1.0.0","metadata":{}}]})).unwrap();
    assert!(
        !plan_metadata(&input, Some("notes"), Platform::Macos)
            .unwrap()
            .is_legacy()
    );
    assert!(matches!(
        plan_metadata(&input, Some("library"), Platform::Macos),
        Err(Error::Package)
    ));
}

#[test]
fn schema_exposes_platforms_and_rejects_unknown_root_fields() {
    let schema = rake_dist::configuration_schema();
    assert!(schema["properties"]["windows"].is_object());
    assert!(schema["properties"]["runtimes"].is_object());
    assert_eq!(schema["additionalProperties"], false);
}
