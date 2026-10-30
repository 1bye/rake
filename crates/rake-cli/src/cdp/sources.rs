//! Read-only source links for captured diagnostic call sites, confined to the workspace.

use super::wire::Error;
use super::wire::Event;
use rake_protocol::inspection::SourceLocation;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct Sources {
    locations: BTreeMap<String, SourceLocation>,
    streams: BTreeMap<String, (String, usize)>,
    next: u64,
    prefix: Option<String>,
    runtime: String,
    catalog: BTreeMap<String, SourceLocation>,
    file_urls: BTreeMap<String, String>,
    pending: BTreeSet<String>,
}

impl Sources {
    pub fn bind(&mut self, base: &str, runtime: &str) {
        self.clear();
        self.prefix = Some(format!("{base}/{runtime}/source/workspace"));
        self.runtime = runtime.into();
    }

    pub fn admit(&mut self, location: &SourceLocation) -> Option<String> {
        if let Some(url) = self.file_urls.get(&location.file) {
            return Some(url.clone());
        }
        if self.catalog.len() >= 128 || self.file_urls.len() >= 128 || location.file.len() > 4096 {
            return None;
        }
        let prefix = self.prefix.as_ref()?;
        let path = crate::source::relative(location).ok()?;
        let mut url = url::Url::parse(prefix).ok()?;
        {
            let mut segments = url.path_segments_mut().ok()?;
            for part in path.split('/') {
                segments.push(part);
            }
        }
        let url = url.to_string();
        self.file_urls.insert(location.file.clone(), url.clone());
        if !self.catalog.contains_key(&url) {
            self.catalog.insert(
                url.clone(),
                SourceLocation {
                    file: path,
                    line: 1,
                    column: 1,
                },
            );
            self.pending.insert(url.clone());
        }
        Some(url)
    }

    pub fn documents(&mut self) -> Vec<Value> {
        self.pending.clear();
        self.catalog
            .keys()
            .map(|url| json!({"frame":self.document(url),"resources":[]}))
            .collect()
    }

    fn document(&self, url: &str) -> Value {
        // These are source-document projections, never native windows or executable scripts.
        json!({"id":format!("rake-source:{url}"),"parentId":"rake","loaderId":self.runtime,
            "name":format!("Rust: {}",self.catalog[url].file),"url":url,"securityOrigin":"rake://source",
            "domainAndRegistry":"","mimeType":"text/plain","secureContextType":"Secure",
            "crossOriginIsolatedContextType":"NotIsolated","gatedAPIFeatures":[]})
    }

    pub fn events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.pending)
            .into_iter()
            .flat_map(|url| {
                let frame = self.document(&url);
                [
                    Event::new(
                        "Page.frameAttached",
                        json!({"frameId":frame["id"],"parentFrameId":"rake"}),
                    ),
                    Event::new(
                        "Page.frameNavigated",
                        json!({"frame":frame,"type":"Navigation"}),
                    ),
                ]
            })
            .collect()
    }

    pub fn capture(&mut self, url: String, location: SourceLocation) {
        self.admit(&location);
        if self.locations.len() >= 128 && !self.locations.contains_key(&url) {
            self.locations.pop_first();
        }
        self.locations.insert(url, location);
    }

    pub fn observe(&mut self, record: &rake_diagnostics::Record) {
        if let Some(source) = &record.source {
            self.admit(&SourceLocation {
                file: source.file.clone(),
                line: source.line,
                column: 1,
            });
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
        if self.prefix.is_some() {
            return Vec::new();
        }
        self.locations
            .keys()
            .map(|url| json!({"url":url,"type":"Document","mimeType":"text/plain"}))
            .collect()
    }

    pub fn content(&self, params: &Value) -> Result<String, Error> {
        let location = params["url"]
            .as_str()
            .and_then(|url| self.catalog.get(url).or_else(|| self.locations.get(url)))
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
        self.catalog.clear();
        self.file_urls.clear();
        self.pending.clear();
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

    #[test]
    fn catalog_deduplicates_files_and_publishes_read_only_documents() {
        let mut sources = Sources::default();
        sources.bind("http://127.0.0.1/token", "runtime-a");
        let mut site = SourceLocation {
            file: "src/cdp/sources.rs".into(),
            line: 12,
            column: 3,
        };
        let first = sources.admit(&site).unwrap();
        site.line = 200;
        assert_eq!(sources.admit(&site).unwrap(), first);
        assert!(first.ends_with("/runtime-a/source/workspace/src/cdp/sources.rs"));
        let events = sources.events();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].method, "Page.frameAttached");
        assert_eq!(events[1].params["frame"]["url"], first);
        assert!(sources.events().is_empty());
        assert!(sources.resources().is_empty());
        let documents = sources.documents();
        assert_eq!(documents.len(), 1);
        assert!(
            documents[0]["frame"]["name"]
                .as_str()
                .unwrap()
                .starts_with("Rust: ")
        );
        assert!(
            sources
                .content(&json!({"url":first}))
                .unwrap()
                .contains("struct Sources")
        );
        assert!(
            sources
                .admit(&SourceLocation {
                    file: "../outside.rs".into(),
                    line: 1,
                    column: 1
                })
                .is_none()
        );
        sources.bind("http://127.0.0.1/token", "runtime-b");
        assert!(sources.documents().is_empty());
        assert!(sources.content(&json!({"url":first})).is_err());
    }
}
