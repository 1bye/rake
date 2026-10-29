//! Covers: facade HTTP type identity and managed-work error conversion.
//! Real: standalone HTTP and protocol contracts. Doubles: none. Requires: none.

#[test]
fn facade_preserves_http_types_and_task_error_conversion() {
    let client: rake_http::HttpClient =
        rake::http::HttpClient::new(rake::http::HttpConfig::new()).unwrap();

    let _: rake::http::HttpClient = client;
    let error: rake::tasks::TaskError = rake::http::Error::Cancelled.into();

    assert_eq!(
        error,
        rake_protocol::tasks::TaskError::Failed("HTTP request cancelled".into())
    );
    let token: rake_http::CancellationToken = rake::http::CancellationToken::new();

    token.cancel();
    assert!(token.is_cancelled());
}
