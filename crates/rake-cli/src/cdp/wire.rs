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
