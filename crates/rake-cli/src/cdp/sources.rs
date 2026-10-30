//! Read-only source links for captured diagnostic call sites, confined to the workspace.

use super::wire::Error;
use rake_protocol::inspection::SourceLocation;
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Sources {
    locations: BTreeMap<String, SourceLocation>,
    streams: BTreeMap<String, (String, usize)>,
    next: u64,
}

impl Sources {
    pub fn capture(&mut self, url: String, location: SourceLocation) {
        if self.locations.len() >= 128 && !self.locations.contains_key(&url) {
            self.locations.pop_first();
        }
        self.locations.insert(url, location);
    }

    pub fn observe(&mut self, record: &rake_diagnostics::Record) {
        if let Some(source) = &record.source {
            let url = super::console::source_url(source);
            if self.locations.len() < 128 || self.locations.contains_key(&url) {
                self.locations.insert(
                    url,
                    SourceLocation {
                        file: source.file.clone(),
                        line: source.line,
                        column: 1,
                    },
                );
            }
        }
    }

    pub fn alias(&mut self, url: String, record: &rake_diagnostics::Record) {
        if let Some(source) = &record.source
            && self.locations.len() < 128
        {
            self.locations.insert(
                url,
                SourceLocation {
                    file: source.file.clone(),
                    line: source.line,
                    column: 1,
                },
            );
        }
    }

    pub fn resources(&self) -> Vec<Value> {
        self.locations
            .keys()
            .map(|url| json!({"url":url,"type":"Document","mimeType":"text/plain"}))
            .collect()
    }

    pub fn content(&self, params: &Value) -> Result<String, Error> {
        let location = params["url"]
            .as_str()
            .and_then(|url| self.locations.get(url))
            .ok_or_else(|| Error::new(-32602, "Source URL was not captured in this session"))?;
        crate::source::read(location).map_err(|error| Error::new(-32000, error.to_string()))
    }

    pub fn stream(&mut self, text: String) -> Result<Value, Error> {
        if self.streams.len() == 8 {
            return Err(Error::new(
                -32000,
                "Close a source stream before opening another",
            ));
        }
        self.next += 1;
        let handle = format!("rake-source-{}", self.next);
        self.streams.insert(handle.clone(), (text, 0));
        Ok(json!({"resource":{"success":true,"httpStatusCode":200,"stream":handle}}))
    }

    pub fn read(&mut self, params: &Value) -> Result<Value, Error> {
        let handle = params["handle"].as_str().unwrap_or_default();
        let (text, cursor) = self
            .streams
            .get_mut(handle)
            .ok_or_else(|| Error::new(-32000, "Source stream expired"))?;
        if let Some(offset) = params["offset"].as_u64() {
            let offset = usize::try_from(offset)
                .unwrap_or(usize::MAX)
                .min(text.len());
            if !text.is_char_boundary(offset) {
                return Err(Error::new(-32602, "Source offset splits a UTF-8 character"));
            }
            *cursor = offset;
        }
        let size = params["size"].as_u64().unwrap_or(65536).clamp(4, 65536) as usize;
        let mut end = cursor.saturating_add(size).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let data = &text[*cursor..end];
        *cursor = end;
        Ok(json!({"data":data,"base64Encoded":false,"eof":end==text.len()}))
    }

    pub fn clear(&mut self) {
        self.locations.clear();
        self.streams.clear();
    }

    pub fn close(&mut self, params: &Value) {
        self.streams
            .remove(params["handle"].as_str().unwrap_or_default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unobserved_sources_are_rejected_and_streams_preserve_unicode() {
        let mut sources = Sources::default();
        assert!(
            sources
                .content(&json!({"url":"rake://source/../../private.rs"}))
                .is_err()
        );
        sources
            .streams
            .insert("rake-source-1".into(), ("café🙂tail".into(), 0));
        let first = sources
            .read(&json!({"handle":"rake-source-1","size":4}))
            .unwrap();
        assert_eq!(first["data"], "caf");
        assert_eq!(first["eof"], false);
        let second = sources
            .read(&json!({"handle":"rake-source-1","size":6}))
            .unwrap();
        assert_eq!(second["data"], "é🙂");
        sources.close(&json!({"handle":"rake-source-1"}));
        assert!(sources.read(&json!({"handle":"rake-source-1"})).is_err());
    }
}
