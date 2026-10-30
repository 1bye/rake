//! Disposable HTTP inspection commands, compiled only with network-test.

use rake::http::{CancellationToken, HttpClient};
use rake::prelude::*;
use std::time::Duration;

#[rake::data(schema)]
#[derive(rake::Command)]
#[command(id = "test.network", output = Output, capability = Headless)]
#[serde(deny_unknown_fields)]
struct Fixture {
    url: String,
    mode: String,
}

#[rake::data(schema)]
struct Output {
    status: u16,
    bytes: usize,
}

pub(super) fn register(commands: &Commands, http: HttpClient) {
    commands
        .register::<Fixture, _, _>(move |fixture, _| {
            let http = http.clone();
            async move {
                let token = CancellationToken::new();
                let request = match fixture.mode.as_str() {
                    "post" => http
                        .post(&fixture.url)
                        .json(&rake::serde_json::json!({"message":"café 你好 🙂","count":3})),
                    "get" | "cancel" | "timeout" | "abandon" => http.get(&fixture.url),
                    _ => {
                        return Err(CommandError::Failed(
                            "Use get, post, cancel, timeout or abandon".into(),
                        ));
                    }
                };
                let request = request
                    .header("x-rake-fixture", "network")
                    .header("authorization", "Bearer fixture-secret")
                    .cancellation(token.clone());
                let request = if fixture.mode == "timeout" {
                    request.timeout(Duration::from_millis(50))
                } else {
                    request
                };
                let response = request
                    .send()
                    .await
                    .map_err(|error| CommandError::Failed(error.to_string()))?;
                let status = response.status();
                if fixture.mode == "abandon" {
                    drop(response);
                    return Ok(Output { status, bytes: 0 });
                }
                if fixture.mode == "cancel" {
                    token.cancel();
                }
                let bytes = response
                    .bytes()
                    .await
                    .map_err(|error| CommandError::Failed(error.to_string()))?;
                Ok(Output {
                    status,
                    bytes: bytes.len(),
                })
            }
        })
        .expect("network fixture registration");
}
