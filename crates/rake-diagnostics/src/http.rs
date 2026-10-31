//! Bounded development body retention, separate from lifecycle diagnostics.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Maximum bytes retained for one request or response body.
pub const BODY_LIMIT: usize = 64 * 1024;
const TOTAL_LIMIT: usize = 1024 * 1024;
const REQUEST_LIMIT: usize = 64;

/// Captured bytes and their completeness; a prefix must never be presented as a full body.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Body {
    /// Original bytes, before text decoding.
    pub bytes: Vec<u8>,
    /// Whether application consumption reached EOF (always true for owned request bodies).
    pub complete: bool,
    /// Whether capture exceeded the per-body bound.
    pub truncated: bool,
    /// Terminal failure or abandonment, when present.
    pub failure: Option<String>,
}

#[derive(Default)]
pub(crate) struct Bodies {
    requests: BTreeMap<u64, (Body, Body)>,
    bytes: usize,
}

impl Bodies {
    pub fn begin(&mut self, id: u64, request: &[u8]) {
        let body = Body {
            bytes: request[..request.len().min(BODY_LIMIT)].to_vec(),
            complete: true,
            truncated: request.len() > BODY_LIMIT,
            failure: None,
        };

        self.bytes += body.bytes.len();
        self.requests.insert(id, (body, Body::default()));
        self.evict();
    }

    pub fn append(&mut self, id: u64, bytes: &[u8]) {
        if let Some((_, body)) = self.requests.get_mut(&id) {
            let count = bytes.len().min(BODY_LIMIT - body.bytes.len());

            body.bytes.extend_from_slice(&bytes[..count]);
            body.truncated |= count < bytes.len();
            self.bytes += count;
            self.evict();
        }
    }

    pub fn finish(&mut self, id: u64, phase: &str) {
        if let Some((_, body)) = self.requests.get_mut(&id) {
            body.complete = phase == "completed";

            if !body.complete {
                body.failure = Some(phase.into());
            }
        }
    }

    pub fn get(&self, id: u64, response: bool) -> Option<Body> {
        self.requests
            .get(&id)
            .map(|(request, reply)| if response { reply } else { request }.clone())
    }

    fn evict(&mut self) {
        while self.requests.len() > REQUEST_LIMIT || self.bytes > TOTAL_LIMIT {
            if let Some((_, (request, response))) = self.requests.pop_first() {
                self.bytes -= request.bytes.len() + response.bytes.len();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eviction_bounds_bytes_and_empty_requests_without_resurrection() {
        let mut bodies = Bodies::default();

        for id in 1..100 {
            bodies.begin(id, &vec![42; BODY_LIMIT]);
            bodies.append(id, &vec![43; BODY_LIMIT]);
        }

        assert!(bodies.bytes <= TOTAL_LIMIT);
        assert!(bodies.get(1, false).is_none());
        bodies.append(1, b"late");
        bodies.finish(1, "completed");
        assert!(bodies.get(1, true).is_none());

        for id in 100..200 {
            bodies.begin(id, b"");
        }

        assert!(bodies.requests.len() <= REQUEST_LIMIT);
    }
}
