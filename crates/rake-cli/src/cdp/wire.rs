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

/// Only the encoded payload is retained in production. Tests can inspect its fields.
pub(super) struct EncodedEvent {
    pub method: String,
    pub text: String,
    #[cfg(test)]
    pub params: Value,
}

impl EncodedEvent {
    pub fn new(event: Event) -> Option<Self> {
        let text = encode_bounded(&event, 1024 * 1024).ok()??;
        Some(Self {
            method: event.method,
            text,
            #[cfg(test)]
            params: event.params,
        })
    }
}

/// Abort serialization before allocating an oversized wire payload.
fn encode_bounded(
    value: &impl Serialize,
    limit: usize,
) -> Result<Option<String>, serde_json::Error> {
    struct Limited {
        bytes: Vec<u8>,
        limit: usize,
        exceeded: bool,
    }
    impl std::io::Write for Limited {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
                self.exceeded = true;
                return Err(std::io::Error::other("CDP payload budget exceeded"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = Limited {
        bytes: Vec::new(),
        limit,
        exceeded: false,
    };
    let result = serde_json::to_writer(&mut output, value);
    if output.exceeded {
        return Ok(None);
    }
    result?;
    Ok(Some(
        String::from_utf8(output.bytes).expect("JSON serializer produces UTF-8"),
    ))
}

/// Count cache metadata without constructing another serialized copy.
pub(super) fn serialized_len(value: &impl Serialize) -> Result<usize, serde_json::Error> {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .ok_or_else(|| std::io::Error::other("size overflow"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Counter(0);
    serde_json::to_writer(&mut count, value)?;
    Ok(count.0)
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
    #[serde(skip)]
    pub kind: ErrorKind,
    pub code: i32,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ErrorKind {
    Protocol,
    RecordingLeaseExpired,
    RecordingClockRestarted,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}
impl std::error::Error for Error {}

impl Error {
    pub fn recording(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            code: -32000,
            message: message.into(),
        }
    }

    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Protocol,
            code,
            message: message.into(),
        }
    }
}

impl Response {
    pub fn encode(&self) -> Result<String, serde_json::Error> {
        if let Some(text) = encode_bounded(self, 1024 * 1024)? {
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
    fn encoding_budget_counts_escaped_utf8_bytes_and_keeps_valid_json() {
        let value = serde_json::json!({"text":"世\n\"🌍"});
        let expected = serde_json::to_string(&value).unwrap();
        assert_eq!(
            encode_bounded(&value, expected.len()).unwrap(),
            Some(expected.clone())
        );
        assert!(
            encode_bounded(&value, expected.len() - 1)
                .unwrap()
                .is_none()
        );
        assert_eq!(serialized_len(&value).unwrap(), expected.len());
        let event = EncodedEvent::new(Event::new("test", value)).unwrap();
        let decoded: Value = serde_json::from_str(&event.text).unwrap();
        assert_eq!(decoded["method"], "test");
        assert_eq!(decoded["params"]["text"], "世\n\"🌍");
    }

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
