//! Shared privacy filters and encoded record budget for native and browser spools.

use super::CrashRecord;
#[cfg(not(target_family = "wasm"))]
use std::fmt;
use std::io;

pub(super) const PRIVATE_MESSAGE: &str = "Rust panic (payload omitted for privacy)";
const MAX_PANIC_MESSAGE_BYTES: usize = 1024;
#[cfg(not(target_family = "wasm"))]
pub(super) const MAX_BACKTRACE_BYTES: usize = 32 * 1024;

#[cfg(not(target_family = "wasm"))]
pub(super) struct TraceText {
    pub(super) text: String,
    pub(super) truncated: bool,
}

#[cfg(not(target_family = "wasm"))]
impl fmt::Write for TraceText {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let available = MAX_BACKTRACE_BYTES.saturating_sub(self.text.len());
        let part = prefix(text, available);
        self.text.push_str(part);
        self.truncated |= part.len() < text.len();

        Ok(())
    }
}

pub(super) fn panic_message(payload: &(dyn std::any::Any + Send), include: bool) -> (String, bool) {
    let message = include
        .then(|| {
            payload
                .downcast_ref::<&'static str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        })
        .flatten();

    match message {
        Some(message) => (
            prefix(message, MAX_PANIC_MESSAGE_BYTES).into(),
            message.len() > MAX_PANIC_MESSAGE_BYTES,
        ),
        None => (PRIVATE_MESSAGE.into(), false),
    }
}

pub(super) fn prefix(text: &str, bytes: usize) -> &str {
    &text[..text.floor_char_boundary(bytes.min(text.len()))]
}

pub(super) fn source_path(path: &str) -> String {
    let absolute = path.starts_with(['/', '\\']) || path.as_bytes().get(1) == Some(&b':');

    if !absolute {
        return path.into();
    }

    let normalized = path.replace('\\', "/");

    if let Some((parent, source)) = normalized.rsplit_once("/src/")
        && let Some(crate_name) = parent.rsplit('/').next().filter(|name| !name.is_empty())
    {
        return format!("{crate_name}/src/{source}");
    }

    normalized.rsplit('/').next().unwrap_or("unknown").into()
}

pub(super) fn encode(record: &mut CrashRecord, limit: usize) -> io::Result<Vec<u8>> {
    loop {
        let bytes = serde_json::to_vec(&*record).map_err(io::Error::other)?;

        if bytes.len() <= limit {
            return Ok(bytes);
        }

        record.truncated = true;

        if let Some(tail) = record.diagnostics.as_mut()
            && !tail.is_empty()
        {
            tail.remove(0);
        } else if record.message != PRIVATE_MESSAGE && !record.message.is_empty() {
            record
                .message
                .truncate(record.message.floor_char_boundary(record.message.len() / 2));
        } else if !record.backtrace.is_empty() {
            record.backtrace.truncate(
                record
                    .backtrace
                    .floor_char_boundary(record.backtrace.len() / 2),
            );
        } else if let Some(location) = record.location.as_mut()
            && !location.is_empty()
        {
            location.truncate(location.floor_char_boundary(location.len() / 2));
        } else if !record.app_version.is_empty() {
            record.app_version.truncate(
                record
                    .app_version
                    .floor_char_boundary(record.app_version.len() / 2),
            );
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "crash metadata exceeds budget",
            ));
        }
    }
}
