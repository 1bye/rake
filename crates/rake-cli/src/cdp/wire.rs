//! Owned CDP envelopes; domain translators only construct events and results.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub(super) struct Request {
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize)]
pub(super) struct Event {
    pub method: String,
    pub params: Value,
}

impl Event {
    pub fn new(method: &str, params: Value) -> Self {
        Self {
            method: method.into(),
            params,
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct Response {
    pub id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Error>,
}

#[derive(Debug, Serialize)]
pub(super) struct Error {
    pub code: i32,
    pub message: String,
}

impl Error {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl Response {
    pub fn encode(&self) -> Result<String, serde_json::Error> {
        let text = serde_json::to_string(self)?;
        if text.len() <= 1024 * 1024 {
            return Ok(text);
        }
        let key = self
            .result
            .as_ref()
            .and_then(|result| result["result"]["value"]["requestKey"].as_str());
        let message = match key {
            Some(key) => format!(
                "CDP reply exceeds 1 MiB; request key {key}; inspect the operation through rake ctl. This command was not resent."
            ),
            None => "CDP reply exceeds 1 MiB; request a smaller capture or property set".into(),
        };

        serde_json::to_string(&Self::new(self.id, Err(Error::new(-32000, message))))
    }

    pub fn new(id: Option<u64>, result: Result<Value, Error>) -> Self {
        match result {
            Ok(result) => Self {
                id,
                result: Some(result),
                error: None,
            },
            Err(error) => Self {
                id,
                result: None,
                error: Some(error),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_replies_keep_the_command_retry_key() {
        let response = Response::new(
            Some(4),
            Ok(
                serde_json::json!({"result":{"value":{"requestKey":"delivery-key","data":"x".repeat(1024*1024)}}}),
            ),
        );
        let encoded = response.encode().unwrap();
        let reply: Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(reply["id"], 4);
        assert_eq!(reply["error"]["code"], -32000);
        assert!(
            reply["error"]["message"]
                .as_str()
                .unwrap()
                .contains("delivery-key")
        );
        assert!(encoded.len() < 1024 * 1024);
    }
}
