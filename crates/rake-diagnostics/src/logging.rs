//! One bounded, redacted line shared by file output and control snapshots.

use std::collections::BTreeMap;
use std::fmt::{self, Write};

use serde_json::Value;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use web_time::{SystemTime, UNIX_EPOCH};

/// Maximum UTF-8 bytes per application log record, including its final newline.
pub const MAX_LOG_RECORD_BYTES: usize = 4096;
const TRUNCATED: &str = "…[truncated]";

fn secret(key: &str) -> bool {
    let key = key.to_ascii_lowercase();

    [
        "password",
        "passwd",
        "secret",
        "token",
        "authorization",
        "cookie",
        "api_key",
        "apikey",
        "credential",
        "private_key",
        "session",
    ]
    .iter()
    .any(|part| key.contains(part))
}

struct Line {
    text: String,
    truncated: bool,
}

impl Write for Line {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let available = (MAX_LOG_RECORD_BYTES - 1).saturating_sub(self.text.len());
        let end = text.floor_char_boundary(available.min(text.len()));

        self.text.push_str(&text[..end]);

        if end != text.len() {
            self.truncated = true;

            return Err(fmt::Error);
        }

        Ok(())
    }
}

pub(super) fn format(
    level: &str,
    target: &str,
    fields: &BTreeMap<String, Value>,
    context: &BTreeMap<String, Value>,
    now: SystemTime,
) -> String {
    let seconds = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let timestamp = OffsetDateTime::from_unix_timestamp(seconds.min(i64::MAX as u64) as i64)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into());

    let mut line = Line {
        text: String::new(),
        truncated: false,
    };

    let _ = write!(line, "{timestamp} {level} {}", target.escape_debug());

    if let Some(message) = fields.get("message").or_else(|| context.get("message")) {
        let _ = write!(line, " {message}");
    }

    for (key, value) in context
        .iter()
        .filter(|(key, _)| !fields.contains_key(*key))
        .chain(fields)
    {
        if line.truncated {
            break;
        }

        if key == "message" {
            continue;
        }

        let _ = write!(line, " {}=", key.escape_debug());

        if secret(key) {
            let _ = line.write_str("[redacted]");
        } else {
            let _ = write!(line, "{value}");
        }
    }

    if line.truncated {
        line.text.truncate(
            line.text
                .floor_char_boundary(MAX_LOG_RECORD_BYTES - 1 - TRUNCATED.len()),
        );
        line.text.push_str(TRUNCATED);
    }

    line.text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_message_target_fields_and_newline_escaping() {
        let fields = [
            ("message".into(), Value::from("first\nsecond\rthird")),
            ("count".into(), Value::from(7)),
            ("detail".into(), Value::from("a\nb")),
        ]
        .into();

        let line = format(
            "INFO",
            "example::worker",
            &fields,
            &BTreeMap::new(),
            UNIX_EPOCH,
        );

        assert_eq!(
            line,
            "1970-01-01T00:00:00Z INFO example::worker \"first\\nsecond\\rthird\" count=7 detail=\"a\\nb\""
        );
        assert_eq!(line.lines().count(), 1);
    }

    #[test]
    fn secret_keys_match_case_insensitive_substrings() {
        for key in [
            "PASSWORD",
            "user_passwd",
            "clientSecret",
            "access_TOKEN",
            "Authorization",
            "Set-Cookie",
            "API_KEY",
            "apiKey",
            "credentials",
            "PRIVATE_KEY",
            "sessionId",
        ] {
            let fields = [(key.into(), Value::from("sensitive-value"))].into();

            let line = format("WARN", "app", &fields, &BTreeMap::new(), UNIX_EPOCH);

            assert!(line.contains("=[redacted]"), "field {key}: {line}");
            assert!(!line.contains("sensitive-value"), "field {key}: {line}");
        }

        let fields = [("count".into(), Value::from(42))].into();

        let line = format("INFO", "app", &fields, &BTreeMap::new(), UNIX_EPOCH);

        assert!(line.contains("count=42"));
    }

    #[test]
    fn truncation_is_marked_utf8_safe_and_at_most_four_kib() {
        let fields = [("message".into(), Value::from("🦀".repeat(4096)))].into();

        let line = format("INFO", "app", &fields, &BTreeMap::new(), UNIX_EPOCH);

        assert!(line.len() < MAX_LOG_RECORD_BYTES);
        assert!(line.ends_with(TRUNCATED));
        assert_eq!(line.lines().count(), 1);
    }
}
