//! Read-only source links for captured diagnostic call sites, confined to the workspace.

use super::wire::Error;
use super::wire::Event;
use rake_protocol::inspection::SourceLocation;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, Weak};

type Catalog = Mutex<BTreeMap<String, SourceLocation>>;

/// Weak grants expire when their owning WebSocket session is dropped.
#[derive(Default)]
pub(super) struct Access(Mutex<Vec<Weak<Catalog>>>);

impl Access {
    pub fn register(&self, sources: &Sources) -> Result<(), Error> {
        let mut catalogs = self
            .0
            .lock()
            .map_err(|_| Error::new(-32000, "Source access unavailable"))?;
        catalogs.retain(|catalog| catalog.strong_count() > 0);
        if catalogs.len() == 32 {
            return Err(Error::new(-32000, "Source session limit reached"));
        }
        catalogs.push(Arc::downgrade(&sources.catalog));
        Ok(())
    }

    pub fn content(&self, url: &str) -> Result<String, Error> {
        let catalogs: Vec<_> = self
            .0
            .lock()
            .map_err(|_| Error::new(-32000, "Source access unavailable"))?
            .iter()
            .filter_map(Weak::upgrade)
            .collect();
        for catalog in catalogs {
            let location = catalog
                .lock()
                .ok()
                .and_then(|catalog| catalog.get(url).cloned());
            if let Some(location) = location {
                return crate::source::read(&location)
                    .map_err(|e| Error::new(-32000, e.to_string()));
            }
        }
        Err(Error::new(-32602, "Source URL expired or was not captured"))
    }
}

#[derive(Default)]
pub(super) struct Sources {
    locations: BTreeMap<String, SourceLocation>,
    streams: BTreeMap<String, (String, usize)>,
    next: u64,
    prefix: Option<String>,
    runtime: String,
    catalog: Arc<Catalog>,
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
        if self.catalog.lock().ok()?.len() >= 128
            || self.file_urls.len() >= 128
            || location.file.len() > 4096
        {
            return None;
        }
        let prefix = self.prefix.as_ref()?;
        let path = crate::source::relative(location).ok()?;
        let url = file_url(prefix, &path)?;
        self.file_urls.insert(location.file.clone(), url.clone());
        let mut catalog = self.catalog.lock().ok()?;
        if !catalog.contains_key(&url) {
            catalog.insert(
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
        let urls: Vec<_> = self
            .catalog
            .lock()
            .map(|catalog| catalog.keys().cloned().collect())
            .unwrap_or_default();
        urls.iter()
            .map(|url| json!({"frame":self.document(url),"resources":[]}))
            .collect()
    }

    fn document(&self, url: &str) -> Value {
        // These are source-document projections, never native windows or executable scripts.
        let path = self
            .catalog
            .lock()
            .ok()
            .and_then(|catalog| catalog.get(url).map(|site| site.file.clone()))
            .unwrap_or_else(|| "Unavailable source".into());
        json!({"id":format!("rake-source:{url}"),"parentId":"rake","loaderId":self.runtime,
            "name":format!("Rust: {}",path),"url":url,"securityOrigin":"rake://source",
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

    fn location(&self, url: &str) -> Result<SourceLocation, Error> {
        self.catalog
            .lock()
            .ok()
            .and_then(|catalog| catalog.get(url).cloned())
            .or_else(|| self.locations.get(url).cloned())
            .ok_or_else(|| Error::new(-32602, "Source URL was not captured in this session"))
    }

    pub fn content(&self, params: &Value) -> Result<String, Error> {
        let location = self.location(params["url"].as_str().unwrap_or_default())?;
        crate::source::read(&location).map_err(|error| Error::new(-32000, error.to_string()))
    }

    pub fn search(&self, params: &Value) -> Result<Value, Error> {
        search(&self.content(params)?, params)
    }

    pub fn site(&self, params: &Value) -> Result<SourceLocation, Error> {
        let mut location = self.location(params["url"].as_str().unwrap_or_default())?;
        let text = self.content(params)?;
        let line = coordinate(params, "lineNumber")?;
        let column = coordinate(params, "columnNumber")?;
        let line_text = text
            .split('\n')
            .nth(line)
            .ok_or_else(|| Error::new(-32602, "Source line is outside the file"))?;
        let mut units = 0;
        let mut chars = 0;
        for character in line_text.chars() {
            if units == column {
                break;
            }
            units += character.len_utf16();
            chars += 1;
        }
        if units != column {
            return Err(Error::new(
                -32602,
                "Source column is outside the line or splits a UTF-16 character",
            ));
        }
        location.line = (line + 1) as u32;
        location.column = (chars + 1) as u32;
        Ok(location)
    }

    pub fn stream(&mut self, text: String) -> Result<Value, Error> {
        if text.len() > 256 * 1024 {
            return Err(Error::new(-32000, "Rust source exceeds 256 KiB"));
        }
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
        if let Some(offset) = params.get("offset") {
            *cursor = offset
                .as_u64()
                .and_then(|v| usize::try_from(v).ok())
                .filter(|v| *v <= text.len() && text.is_char_boundary(*v))
                .ok_or_else(|| {
                    Error::new(-32602, "Invalid source offset or split UTF-8 character")
                })?;
        }
        let size = params
            .get("size")
            .map_or(Some(65536), Value::as_u64)
            .filter(|size| *size > 0)
            .ok_or_else(|| Error::new(-32602, "Invalid source stream size"))?
            .min(65536) as usize;
        let mut end = cursor.saturating_add(size).min(text.len());
        while end > *cursor && !text.is_char_boundary(end) {
            end -= 1;
        }
        if end == *cursor && end < text.len() {
            end += text[end..]
                .chars()
                .next()
                .expect("nonempty UTF-8 tail")
                .len_utf8();
        }
        let data = &text[*cursor..end];
        *cursor = end;
        Ok(json!({"data":data,"base64Encoded":false,"eof":end==text.len()}))
    }

    pub fn clear(&mut self) {
        self.locations.clear();
        if let Ok(mut catalog) = self.catalog.lock() {
            catalog.clear();
        }
        self.file_urls.clear();
        self.pending.clear();
        self.streams.clear();
    }

    pub fn close(&mut self, params: &Value) {
        self.streams
            .remove(params["handle"].as_str().unwrap_or_default());
    }
}

fn coordinate(params: &Value, key: &str) -> Result<usize, Error> {
    params
        .get(key)
        .map_or(Some(0), Value::as_u64)
        .and_then(|v| usize::try_from(v).ok())
        .ok_or_else(|| Error::new(-32602, format!("{key} must be a nonnegative integer")))
}

fn search(text: &str, params: &Value) -> Result<Value, Error> {
    let query = params["query"]
        .as_str()
        .filter(|query| !query.is_empty() && query.len() <= 4096)
        .ok_or_else(|| Error::new(-32602, "Source query must contain 1–4096 bytes"))?;
    let flag = |key| {
        params
            .get(key)
            .map_or(Some(false), Value::as_bool)
            .ok_or_else(|| Error::new(-32602, format!("{key} must be a boolean")))
    };
    let pattern = if flag("isRegex")? {
        query.to_owned()
    } else {
        regex::escape(query)
    };
    let regex = regex::RegexBuilder::new(&pattern)
        .case_insensitive(!flag("caseSensitive")?)
        .size_limit(256 * 1024)
        .build()
        .map_err(|error| Error::new(-32602, error.to_string()))?;
    let mut matches = Vec::new();
    for (line, content) in text.split('\n').enumerate() {
        let content = content.strip_suffix('\r').unwrap_or(content);
        if regex.is_match(content) {
            if matches.len() == 1000 {
                return Err(Error::new(
                    -32000,
                    "Source search exceeds 1000 matching lines; narrow the query or search the opened file",
                ));
            }
            matches.push(json!({"lineNumber":line,"lineContent":content}));
        }
    }
    Ok(json!({"result":matches}))
}

fn file_url(prefix: &str, path: &str) -> Option<String> {
    let mut url = url::Url::parse(prefix).ok()?;
    {
        let mut segments = url.path_segments_mut().ok()?;
        for part in path.split('/') {
            segments.push(part);
        }
    }
    Some(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_preserves_line_numbers_and_rejects_partial_results() {
        let text = "first\r\ncafé 🙂\nCafé\n";
        assert_eq!(
            search(text, &json!({"query":"CAFÉ"})).unwrap()["result"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            search(text, &json!({"query":"café", "caseSensitive":true})).unwrap()["result"][0]["lineNumber"],
            1
        );
        assert_eq!(
            search(
                text,
                &json!({"query":"^C.*", "isRegex":true,"caseSensitive":true})
            )
            .unwrap()["result"][0]["lineNumber"],
            2
        );
        assert!(search(text, &json!({"query":"[", "isRegex":true})).is_err());
        assert!(search(text, &json!({"query":""})).is_err());
        assert!(search(&"x\n".repeat(1001), &json!({"query":"x"})).is_err());
    }

    #[test]
    fn source_grants_expire_and_streams_validate_unicode_offsets() {
        let access = Access::default();
        let mut sources = Sources::default();
        sources.bind("http://127.0.0.1/token", "runtime");
        access.register(&sources).unwrap();
        let url = sources
            .admit(&SourceLocation {
                file: "src/cdp/sources.rs".into(),
                line: 1,
                column: 1,
            })
            .unwrap();
        assert!(access.content(&url).unwrap().contains("struct Sources"));
        assert!(
            access
                .content(&url.replace("sources.rs", "session.rs"))
                .is_err()
        );
        let handle = sources.stream("é🙂文".into()).unwrap()["resource"]["stream"].clone();
        assert_eq!(
            sources.read(&json!({"handle":handle,"size":1})).unwrap()["data"],
            "é"
        );
        assert!(sources.read(&json!({"handle":handle,"offset":1})).is_err());
        assert!(sources.read(&json!({"handle":handle,"size":0})).is_err());
        assert!(sources.stream("x".repeat(256 * 1024 + 1)).is_err());
        let encoded = file_url("http://localhost/token/workspace", "src/café #?.rs").unwrap();
        assert!(encoded.ends_with("src/caf%C3%A9%20%23%3F.rs"));
        assert!(url::Url::parse(&encoded).unwrap().fragment().is_none());
        drop(sources);
        assert!(access.content(&url).is_err());
    }

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
