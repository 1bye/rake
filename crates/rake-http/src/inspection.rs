//! Development-only HTTP metadata. Credentials stay redacted in retained diagnostics.

use reqwest::header::HeaderMap;
use serde_json::{Value, json};
use url::Url;

pub(super) fn url(url: &Url) -> String {
    let mut result = url.clone();
    let _ = result.set_username("");
    let _ = result.set_password(None);
    result.set_fragment(None);
    if url.query().is_some() {
        result
            .query_pairs_mut()
            .clear()
            .extend_pairs(url.query_pairs().map(|(key, value)| {
                let value = if sensitive(&key) {
                    "[redacted]".into()
                } else {
                    value
                };
                (key, value)
            }));
    }
    result.into()
}

fn sensitive(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "authorization"
            | "proxy-authorization"
            | "cookie"
            | "set-cookie"
            | "credential"
            | "secret"
            | "password"
            | "token"
            | "access_token"
            | "refresh_token"
            | "api_key"
            | "apikey"
            | "x-api-key"
    )
}

pub(super) fn headers(headers: &HeaderMap) -> Value {
    let mut result = serde_json::Map::new();
    let mut remaining: usize = 16 * 1024;
    for (name, value) in headers {
        let value = if sensitive(name.as_str()) {
            "[redacted]".into()
        } else {
            String::from_utf8_lossy(value.as_bytes()).into_owned()
        };
        if name.as_str().len() + value.len() > remaining {
            result.insert(
                "rake-capture-truncated".into(),
                json!("header capture exceeds 16 KiB"),
            );
            break;
        }
        remaining -= name.as_str().len() + value.len();
        let entry = result.entry(name.as_str()).or_insert_with(|| json!(""));
        let previous = entry.as_str().unwrap_or_default();
        *entry = json!(if previous.is_empty() {
            value
        } else {
            format!("{previous}\n{value}")
        });
    }
    Value::Object(result)
}
