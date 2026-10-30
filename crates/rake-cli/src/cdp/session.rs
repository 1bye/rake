//! One connection owns its cursors, enabled domains and bounded event queue.

use super::wire::{Error, Event, Request, Response};
use super::{Result, TIMEOUT};
use crate::terminal::query;
use rake_client::local::Instance;

use rake_protocol::control::{
    Command, Message as RuntimeMessage, PROTOCOL_VERSION, Query, Reply, Request as RuntimeRequest,
};

use serde_json::{Value, json};
use std::collections::{BTreeSet, VecDeque};
use std::net::TcpStream;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tungstenite::{Message, WebSocket};

const INTERVAL: Duration = Duration::from_millis(250);
const QUEUE_LIMIT: usize = 256;
// Browser proxies must preserve the future cursor exactly when reading the high-water mark.
const FRAME_HIGH_WATER_CURSOR: u64 = (1 << 53) - 1;

struct Session {
    instance: Instance,
    cursor: u64,
    network_cursor: u64,
    network_paused: bool,
    network_epoch: Option<f64>,
    network_bodies: super::network::Bodies,
    domains: BTreeSet<String>,
    outbound: VecDeque<(Event, usize)>,
    outbound_bytes: usize,
    dropped: u64,
    epoch_seconds: Option<f64>,
    exception_sequence: u64,
    discard_diagnostics: bool,
    objects: super::console::Objects,
    sources: super::sources::Sources,
    source_base: Option<String>,
    source_tree_loaded: bool,
    loss: super::console::Loss,
    process_output: super::console::ProcessOutput,
    dom: super::dom::Dom,
    picking: bool,
    interaction_used: bool,
    highlight: Option<i32>,
    last_hover: Option<(String, String)>,
    last_pick_sequence: u64,
    tracked_style_node: Option<i32>,
    edit_sheets: BTreeSet<i32>,
    frame_trace: super::frames::Trace,
    memory: super::memory::Memory,
    storage: super::storage::Storage,
    pending_trace_complete: Option<Value>,
    detached: Option<String>,
}

impl Session {
    fn new(instance: Instance, output: Option<std::path::PathBuf>) -> Self {
        Self {
            instance,
            cursor: 0,
            network_cursor: 0,
            network_paused: false,
            network_epoch: None,
            network_bodies: super::network::Bodies::default(),
            domains: BTreeSet::new(),
            outbound: VecDeque::new(),
            outbound_bytes: 0,
            dropped: 0,
            epoch_seconds: None,
            exception_sequence: 0,
            discard_diagnostics: false,
            objects: super::console::Objects::default(),
            sources: super::sources::Sources::default(),
            source_base: None,
            source_tree_loaded: false,
            loss: super::console::Loss::default(),
            process_output: super::console::ProcessOutput::new(output),
            dom: super::dom::Dom::default(),
            picking: false,
            interaction_used: false,
            highlight: None,
            last_hover: None,
            last_pick_sequence: 0,
            tracked_style_node: None,
            edit_sheets: BTreeSet::new(),
            frame_trace: super::frames::Trace::default(),
            memory: super::memory::Memory::default(),
            storage: super::storage::Storage::default(),
            pending_trace_complete: None,
            detached: None,
        }
    }

    fn event(&mut self, event: Event) {
        let previous_dropped = self.dropped;
        let bytes = serde_json::to_vec(&event)
            .map(|bytes| bytes.len())
            .unwrap_or(usize::MAX);

        if self.outbound.len() == QUEUE_LIMIT
            || bytes > 1024 * 1024
            || self.outbound_bytes.saturating_add(bytes) > 1024 * 1024
        {
            self.dropped = self.dropped.saturating_add(1);
        } else {
            self.outbound_bytes += bytes;
            self.outbound.push_back((event, bytes));
        }
        if previous_dropped == 0 && self.dropped > 0 {
            self.sources.republish();
        }
    }

    fn handle(&mut self, request: &Request) -> std::result::Result<Value, Error> {
        if !request.params.is_null() && !request.params.is_object() {
            return Err(Error::new(-32602, "params must be an object"));
        }

        let method = request.method.as_str();
        if let Some(reason) = &self.detached
            && !matches!(
                method,
                "IO.read" | "IO.close" | "Tracing.end" | "Rake.getPerformanceSummary"
            )
        {
            return Err(Error::new(
                -32000,
                format!("Runtime disconnected: {reason}"),
            ));
        }

        match method {
            "DOMStorage.enable" => {
                self.domains.insert("DOMStorage".into());
                Ok(json!({}))
            }
            "DOMStorage.disable" => {
                self.domains.remove("DOMStorage");
                self.storage.clear();
                Ok(json!({}))
            }
            "Storage.getStorageKeyForFrame" => {
                super::storage::Storage::storage_key(&request.params)
            }
            "Rake.getStorage"
            | "DOMStorage.getDOMStorageItems"
            | "DOMStorage.setDOMStorageItem"
            | "DOMStorage.removeDOMStorageItem"
            | "DOMStorage.clear" => {
                let (result, events) =
                    self.storage
                        .handle(&self.instance, method, &request.params)?;
                if self.domains.contains("DOMStorage") {
                    for event in events {
                        self.event(event);
                    }
                }
                Ok(result)
            }
            "HeapProfiler.enable" => Ok(json!({})),
            "HeapProfiler.disable" => {
                self.stop_memory();
                self.memory = super::memory::Memory::default();
                Ok(json!({}))
            }
            "Runtime.getIsolateId" => Ok(json!({"id":self.instance.info.runtime_id.to_string()})),
            "Runtime.getHeapUsage" => Ok(json!({"usedSize":0,"totalSize":0})), // Native host has no JavaScript heap.
            "HeapProfiler.startSampling" => {
                self.start_memory(&request.params)?;
                Ok(json!({}))
            }
            "HeapProfiler.getSamplingProfile" | "HeapProfiler.stopSampling" => {
                let mut profile = self.memory_profile();
                if method == "HeapProfiler.stopSampling" {
                    self.stop_memory();
                    if let Ok(profile) = &mut profile {
                        profile["rake"]["recording"] = json!(false);
                    }
                }
                Ok(json!({"profile":profile?}))
            }
            "Rake.getMemorySummary" => Ok(self.memory_status()),
            "Rake.stopMemory" => {
                self.stop_memory();
                self.memory = super::memory::Memory::default();
                self.event(Event::new("HeapProfiler.resetProfiles", json!({})));
                Ok(self.memory_status())
            }
            "HeapProfiler.takeHeapSnapshot" => {
                if !self.memory.active {
                    self.start_memory(&json!({}))?;
                }
                let capture = self.memory_capture(false)?;
                if !capture.recording {
                    return Err(Error::new(
                        -32000,
                        "Native memory lease expired; use rake.stopMemory() before a new capture",
                    ));
                }
                let frames = self.memory_frames(&capture);
                let text = self.memory.heap_snapshot(&capture, &frames)?;
                let mut events = Vec::new();
                if request.params["reportProgress"] == true {
                    // Chrome prepares its worker on finished progress, before chunks arrive.
                    events.push(Event::new(
                        "HeapProfiler.reportHeapSnapshotProgress",
                        json!({"done":1,"total":1,"finished":true}),
                    ));
                }
                let mut offset = 0;
                while offset < text.len() {
                    let mut end = (offset + 32 * 1024).min(text.len());
                    while !text.is_char_boundary(end) {
                        end -= 1;
                    }
                    events.push(Event::new(
                        "HeapProfiler.addHeapSnapshotChunk",
                        json!({"chunk":&text[offset..end]}),
                    ));
                    offset = end;
                }
                let bytes: usize = events
                    .iter()
                    .map(|event| serde_json::to_vec(event).map_or(usize::MAX, |bytes| bytes.len()))
                    .sum();
                if self.outbound.len() + events.len() > QUEUE_LIMIT
                    || self.outbound_bytes.saturating_add(bytes) > 1024 * 1024
                {
                    return Err(Error::new(
                        -32000,
                        "Outbound queue busy; retry the heap snapshot",
                    ));
                }
                for event in events {
                    self.event(event);
                }
                Ok(json!({}))
            }
            "HeapProfiler.collectGarbage" => Err(Error::new(
                -32601,
                "Rust memory is released by its owners; garbage collection is unavailable",
            )),
            "Accessibility.enable" => {
                self.domains.insert("Accessibility".into());
                Ok(json!({}))
            }
            "Accessibility.disable" => {
                self.domains.remove("Accessibility");
                Ok(json!({}))
            }
            "Accessibility.getRootAXNode"
            | "Accessibility.getFullAXTree"
            | "Accessibility.getPartialAXTree"
            | "Accessibility.getChildAXNodes"
            | "Accessibility.getAXNodeAndAncestors" => {
                self.refresh_dom()?;
                self.dom.accessibility(method, &request.params)
            }
            "Rake.getPerformanceSummary" => Ok(self.frame_trace.summary()),
            "Tracing.getCategories" => Ok(
                json!({"categories":["devtools.timeline","rake.frames","rake.callbacks","rake.causal"]}),
            ),
            "Tracing.start" => {
                if self.pending_trace_complete.is_some() {
                    return Err(Error::new(-32000, "Stop the previous recording first"));
                }
                if !self
                    .instance
                    .info
                    .commands
                    .iter()
                    .any(|command| command == "record_frames")
                {
                    return Err(Error::new(-32000, "Frame recording is unavailable"));
                }
                self.frame_trace.begin(&request.params)?;
                if let Err(error) = self.control(Command::RecordFrames {
                    enabled: true,
                    lease_ms: 5000,
                }) {
                    self.frame_trace.active = false;
                    return Err(error);
                }
                let baseline = query(
                    &self.instance,
                    Query::Frames {
                        after: FRAME_HIGH_WATER_CURSOR,
                    },
                    TIMEOUT,
                );
                match baseline {
                    Ok(Reply::Frames(baseline)) => {
                        self.frame_trace.cursor = baseline.cursor;
                        if let Some(clock) = &baseline.clock {
                            self.frame_trace.causal.calibrate(clock);
                        }
                        self.frame_trace.source_prefix = self.source_base.as_ref().map(|base| {
                            format!("{base}/{}/source/frame", self.instance.info.runtime_id)
                        });
                    }
                    other => {
                        let _ = self.control(Command::RecordFrames {
                            enabled: false,
                            lease_ms: 0,
                        });
                        self.frame_trace.active = false;
                        return Err(Error::new(
                            -32000,
                            format!("Frame baseline unavailable: {other:?}"),
                        ));
                    }
                }
                Ok(json!({}))
            }
            "Tracing.end" => {
                if !self.frame_trace.active {
                    if let Some(complete) = self.pending_trace_complete.take() {
                        self.event(Event::new("Tracing.tracingComplete", complete));
                        return Ok(json!({}));
                    }
                    return if self.frame_trace.has_stream() {
                        Ok(json!({}))
                    } else {
                        Err(Error::new(-32000, "Frame tracing is not active"))
                    };
                }
                self.finish_trace(None);
                Ok(json!({}))
            }
            "IO.read"
                if !request.params["handle"]
                    .as_str()
                    .is_some_and(|h| h.starts_with("rake-source-")) =>
            {
                self.frame_trace.read(&request.params)
            }
            "IO.close"
                if request.params["handle"]
                    .as_str()
                    .is_some_and(|handle| handle.starts_with("rake-frames-")) =>
            {
                self.frame_trace.close(&request.params)?;
                Ok(json!({}))
            }
            "Runtime.evaluate" => {
                let expression = request.params["expression"]
                    .as_str()
                    .ok_or_else(|| Error::new(-32602, "expression required"))?;
                let expression = expression.trim().trim_end_matches(';').trim();
                if request
                    .params
                    .get("contextId")
                    .is_some_and(|id| id.as_u64() != Some(1))
                    || request.params.get("uniqueContextId").is_some_and(|id| {
                        id.as_str() != Some(&self.instance.info.runtime_id.to_string())
                    })
                {
                    return Err(Error::new(
                        -32602,
                        "Execution context is stale or unavailable",
                    ));
                }

                if expression == "$0" {
                    self.refresh_dom()?;
                    let node = self.dom.inspected_node()?;
                    let object = self.dom.resolve_node(
                        &json!({"nodeId":node,"objectGroup":request.params["objectGroup"]}),
                    )?;
                    return Ok(json!({"result":object["object"]}));
                }
                let read_only = if expression == "rake.memory()" {
                    Some(self.memory_status())
                } else if expression == "rake.sources()" {
                    Some(self.sources.status())
                } else if expression == "rake.help()" {
                    Some(super::evaluate::help())
                } else if expression == "rake.commands()" {
                    Some(super::evaluate::registry(&self.instance)?)
                } else if expression == "rake" {
                    Some(super::evaluate::help())
                } else if !expression.contains('(')
                    && expression.split('.').all(|part| {
                        !part.is_empty() && part.chars().all(|c| c.is_alphanumeric() || c == '_')
                    })
                {
                    super::evaluate::completion(
                        expression,
                        &super::evaluate::names(&self.instance)?,
                    )
                } else {
                    None
                };
                if let Some(value) = read_only {
                    let remote = if request.params["returnByValue"] == true {
                        json!({"type":"object","value":value})
                    } else {
                        self.objects
                            .remote_in(value, request.params["objectGroup"].as_str())
                    };
                    return Ok(json!({"result":remote}));
                }
                if request.params["throwOnSideEffect"] == true {
                    return Err(Error::new(
                        -32000,
                        "Typed commands mutate state; side-effect-free/eager evaluation is unavailable",
                    ));
                }

                let outcome: std::result::Result<Value, Error> = (|| {
                    let value = if expression == "rake.stopMemory()" {
                        self.handle(&Request {
                            id: request.id,
                            method: "Rake.stopMemory".into(),
                            params: json!({}),
                        })?
                    } else if let Some(argument) = expression
                        .strip_prefix("rake.command(")
                        .and_then(|v| v.strip_suffix(')'))
                    {
                        let id: String = serde_json::from_str(argument).map_err(|_| {
                            Error::new(-32602, "Command name must be a JSON string")
                        })?;
                        super::evaluate::registry(&self.instance)?
                            .as_array()
                            .unwrap()
                            .iter()
                            .find(|c| c["id"] == id)
                            .cloned()
                            .ok_or_else(|| {
                                Error::new(-32602, format!("Unregistered command: {id}"))
                            })?
                    } else if let Some(argument) = expression
                        .strip_prefix("rake.operation(")
                        .and_then(|v| v.strip_suffix(')'))
                    {
                        let token: Value = serde_json::from_str(argument).map_err(|_| {
                            Error::new(-32602, "Operation observation requires a JSON token")
                        })?;
                        super::evaluate::observation(&self.instance, &token)?
                    } else if let Some(argument) = expression
                        .strip_prefix("rake.source(")
                        .and_then(|value| value.strip_suffix(')'))
                    {
                        let selector = self.source_selector(argument)?;
                        let location = self.source_location(&selector)?;
                        self.source_descriptor(&location)
                    } else if let Some(argument) = expression
                        .trim()
                        .trim_end_matches(';')
                        .trim()
                        .strip_prefix("rake.openSource(")
                        .and_then(|value| value.strip_suffix(')'))
                    {
                        let selector = self.source_selector(argument)?;
                        let location = self.source_location(&selector)?;
                        crate::source::open(&location)
                            .map_err(|error| Error::new(-32000, error.to_string()))?
                    } else if let Some(method) = match expression {
                        "rake.resetStyles($0)" => Some("Rake.resetStyles"),
                        "rake.undoStyles($0)" => Some("Rake.undoStyles"),
                        "rake.resetText($0)" => Some("Rake.resetText"),
                        "rake.undoText($0)" => Some("Rake.undoText"),
                        _ => None,
                    } {
                        let node = self.dom.inspected_node()?;
                        self.handle(&Request {
                            id: request.id,
                            method: method.into(),
                            params: json!({"nodeId":node}),
                        })?
                    } else {
                        super::evaluate::execute(&self.instance, expression)?
                    };

                    let remote = if request.params["returnByValue"] == true {
                        json!({"type":"object","value":value})
                    } else {
                        let status = value["status"].as_str().map(str::to_owned);
                        let id = value["observe"]["id"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned();
                        let mut remote = self
                            .objects
                            .remote_in(value, request.params["objectGroup"].as_str());
                        if let Some(status) = status {
                            remote["description"] = json!(format!("Command {status} · {id}"));
                        }
                        remote
                    };

                    Ok(json!({"result":remote}))
                })();
                Ok(match outcome {
                    Ok(value) => value,
                    Err(error) => {
                        self.exception_sequence += 1;
                        super::console::exception(error, self.exception_sequence)
                    }
                })
            }
            "Runtime.globalLexicalScopeNames" => Ok(
                json!({"names":super::evaluate::lexical_names(&super::evaluate::names(&self.instance)?)}),
            ),
            "Runtime.compileScript" => {
                let expression = request.params["expression"]
                    .as_str()
                    .ok_or_else(|| Error::new(-32602, "expression required"))?;

                if !matches!(
                    expression.trim(),
                    "rake.help()"
                        | "rake.memory()"
                        | "rake.stopMemory()"
                        | "rake.sources()"
                        | "rake.commands()"
                        | "rake.resetText($0)"
                        | "rake.undoText($0)"
                        | "rake.resetStyles($0)"
                        | "rake.undoStyles($0)"
                ) {
                    if let Some(argument) = expression
                        .trim()
                        .strip_prefix("rake.command(")
                        .and_then(|v| v.strip_suffix(')'))
                    {
                        serde_json::from_str::<String>(argument).map_err(|_| {
                            Error::new(-32602, "Command name must be a JSON string")
                        })?;
                    } else if let Some(argument) = expression
                        .strip_prefix("rake.source(")
                        .and_then(|value| value.strip_suffix(')'))
                    {
                        self.source_selector(argument)?;
                    } else if let Some(argument) = expression
                        .trim()
                        .strip_prefix("rake.operation(")
                        .and_then(|v| v.strip_suffix(')'))
                    {
                        serde_json::from_str::<Value>(argument).map_err(|_| {
                            Error::new(-32602, "Operation observation requires JSON")
                        })?;
                    } else {
                        super::evaluate::validate(&self.instance, expression)?;
                    }
                }

                if request.params["persistScript"] == true {
                    return Err(Error::new(
                        -32000,
                        "Persisted scripts are unavailable; use Runtime.evaluate for a typed command",
                    ));
                }

                Ok(json!({}))
            }
            "DOM.enable" | "CSS.enable" => {
                let fresh = self
                    .domains
                    .insert(method.split('.').next().unwrap_or_default().into());

                if method == "CSS.enable" && fresh {
                    self.event(Event::new("CSS.styleSheetAdded", json!({"header":{
                        "styleSheetId":"rake-resolved", "frameId":"rake", "sourceURL":"rake://resolved-styles",
                        "origin":"regular", "title":"GPUI resolved styles", "disabled":false,
                        "isInline":false, "isMutable":false, "isConstructed":true,
                        "startLine":0, "startColumn":0, "length":0
                    }})));
                }

                if method == "DOM.enable" {
                    self.warning("Rake exposes measured bounds and absolute-edge box geometry. Auto/percentage edges and opaque elements may lack a complete box model. Style rules show observed GPUI origins, not a browser cascade or individual Rust declaration locations.".into());
                }

                Ok(json!({}))
            }
            "DOM.disable" | "CSS.disable" => {
                self.domains
                    .remove(method.split('.').next().unwrap_or_default());

                Ok(json!({}))
            }
            "DOM.getDocument" => {
                self.refresh_dom()?;
                self.domains.insert("DOM".into());

                Ok(json!({"root":self.dom.document(&request.params)?}))
            }
            "DOM.requestChildNodes" => {
                let events = self.dom.request_children(&request.params)?;
                self.child_events(events)?;

                Ok(json!({}))
            }
            "DOM.performSearch" => {
                self.refresh_dom()?;
                let query = request.params["query"]
                    .as_str()
                    .ok_or_else(|| Error::new(-32602, "query required"))?;
                self.dom.search(query)
            }
            "DOM.getOuterHTML" => {
                self.refresh_dom()?;
                self.dom.outer_html(&request.params)
            }
            "DOM.getSearchResults" => {
                self.refresh_dom()?;
                let (result, events) = self.dom.search_results(&request.params)?;
                self.child_events(events)?;
                Ok(result)
            }
            "DOM.discardSearchResults" => self.dom.discard_search(&request.params),
            "DOM.setNodeValue" | "Rake.resetText" | "Rake.undoText" => {
                self.refresh_dom()?;
                let edit = match method {
                    "Rake.resetText" => rake_protocol::inspection::TextEdit::Reset {},
                    "Rake.undoText" => rake_protocol::inspection::TextEdit::Undo {},
                    _ => rake_protocol::inspection::TextEdit::Set {
                        text: request.params["value"]
                            .as_str()
                            .ok_or_else(|| Error::new(-32602, "Text value required"))?
                            .into(),
                    },
                };
                self.control(self.dom.text_command(
                    &request.params,
                    edit,
                    method != "DOM.setNodeValue",
                )?)?;
                // Character-data events follow the actual GPUI draw, including layout and AX updates.
                Ok(json!({}))
            }
            "DOM.setInspectedNode" => self.dom.set_inspected_node(&request.params),
            "DOM.focus" | "DOM.scrollIntoViewIfNeeded" => {
                // Map the retained CDP identity to the latest exact runtime capture.
                self.refresh_dom()?;
                self.control(self.dom.navigation_command(method, &request.params)?)?;
                Ok(json!({}))
            }
            "Rake.getSources" => Ok(self.sources.status()),
            "Rake.getSourceLocation" => {
                let location = self.source_location(&request.params)?;
                Ok(self.source_descriptor(&location))
            }
            "Rake.openSource" => {
                let location = self.source_location(&request.params)?;
                crate::source::open(&location)
                    .map_err(|error| Error::new(-32000, error.to_string()))
            }
            "DOM.resolveNode" => {
                self.refresh_dom()?;
                self.dom.resolve_node(&request.params)
            }
            "DOM.requestNode" => {
                let object = request.params["objectId"]
                    .as_str()
                    .ok_or_else(|| Error::new(-32602, "objectId required"))?;
                let node = self.dom.object_node(object)?;
                let (_, events) = self.dom.push_nodes(&json!({"backendNodeIds":[node]}))?;
                self.child_events(events)?;
                Ok(json!({"nodeId":node}))
            }
            "Runtime.callFunctionOn" => {
                self.refresh_dom()?;
                self.control(self.dom.call_navigation(&request.params)?)?;
                Ok(json!({"result":{"type":"undefined"}}))
            }
            "Page.bringToFront" => Ok(json!({})), // Focus navigation activates the actual target window.
            "DOM.getBoxModel"
            | "DOM.describeNode"
            | "DOM.getAttributes"
            | "CSS.getInlineStylesForNode"
            | "CSS.getMatchedStylesForNode"
            | "CSS.getComputedStyleForNode" => {
                if let Some(node) = request.params["nodeId"]
                    .as_i64()
                    .and_then(|id| i32::try_from(id).ok())
                    && node != 1
                {
                    self.load_details(node)?;
                }
                if method == "CSS.getMatchedStylesForNode"
                    && let Some(node) = request.params["nodeId"]
                        .as_i64()
                        .and_then(|id| i32::try_from(id).ok())
                    && self.dom.editable(node)
                    && self.edit_sheets.insert(node)
                {
                    self.event(Event::new("CSS.styleSheetAdded", json!({"header":{
                        "styleSheetId":format!("rake-edit-{node}"),"frameId":"rake","sourceURL":"rake://devtools/base-overrides",
                        "origin":"regular","title":"DevTools base overrides","disabled":false,"isInline":true,"isMutable":true,
                        "isConstructed":false,"startLine":0,"startColumn":0,"length":self.dom.edit_text(node)?.len(),"endLine":0,"endColumn":0
                    }})));
                }
                self.dom.handle(method, &request.params)
            }
            "CSS.getStyleSheetText"
                if request.params["styleSheetId"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("rake-edit-")) =>
            {
                let node = edit_node(&request.params["styleSheetId"])?;
                self.load_details(node)?;
                Ok(json!({"text":self.dom.edit_text(node)?}))
            }
            "CSS.setStyleTexts"
            | "CSS.setStyleSheetText"
            | "CSS.setEffectivePropertyValueForNode"
            | "Rake.resetStyles"
            | "Rake.undoStyles" => {
                self.refresh_dom()?;
                let (node, edit) = if method == "CSS.setEffectivePropertyValueForNode" {
                    let node = request.params["nodeId"]
                        .as_i64()
                        .and_then(|id| i32::try_from(id).ok())
                        .ok_or_else(|| Error::new(-32602, "nodeId required"))?;
                    let property = request.params["propertyName"]
                        .as_str()
                        .ok_or_else(|| Error::new(-32602, "propertyName required"))?;
                    let value = request.params["value"]
                        .as_str()
                        .ok_or_else(|| Error::new(-32602, "value required"))?;
                    (
                        node,
                        rake_protocol::inspection::StyleEdit::Set {
                            declaration: rake_protocol::inspection::StyleDeclaration {
                                property: property.into(),
                                value: value.into(),
                                enabled: true,
                            },
                        },
                    )
                } else if method.starts_with("Rake.") {
                    let node = request.params["nodeId"]
                        .as_i64()
                        .and_then(|id| i32::try_from(id).ok())
                        .ok_or_else(|| Error::new(-32602, "nodeId required"))?;
                    (
                        node,
                        if method == "Rake.resetStyles" {
                            rake_protocol::inspection::StyleEdit::Reset
                        } else {
                            rake_protocol::inspection::StyleEdit::Undo
                        },
                    )
                } else {
                    let edit = if method == "CSS.setStyleTexts" {
                        let edits = request.params["edits"]
                            .as_array()
                            .filter(|edits| edits.len() == 1)
                            .ok_or_else(|| {
                                Error::new(
                                    -32602,
                                    "Exactly one style edit per request is supported",
                                )
                            })?;
                        &edits[0]
                    } else {
                        &request.params
                    };
                    let node = edit_node(&edit["styleSheetId"])?;
                    self.load_details(node)?;
                    let text = edit["text"]
                        .as_str()
                        .ok_or_else(|| Error::new(-32602, "text required"))?;
                    let text = if method == "CSS.setStyleTexts" {
                        super::editing::replace_range(
                            &self.dom.edit_text(node)?,
                            &edit["range"],
                            text,
                        )?
                    } else {
                        text.into()
                    };
                    (
                        node,
                        rake_protocol::inspection::StyleEdit::Replace {
                            declarations: super::editing::parse(&text)?,
                        },
                    )
                };
                self.control(self.dom.edit_command(node, edit)?)?;
                self.refresh_dom()?;
                self.load_details(node)?;
                self.event(Event::new(
                    "CSS.styleSheetChanged",
                    json!({"styleSheetId":format!("rake-edit-{node}")}),
                ));
                if method == "CSS.setStyleTexts" {
                    Ok(json!({"styles":[self.dom.edit_style(node)?]}))
                } else {
                    Ok(json!({}))
                }
            }
            "CSS.getStyleSheetText" if request.params["styleSheetId"] == "rake-resolved" => {
                Ok(json!({"text":self.dom.style_sheet_text()}))
            }
            "CSS.trackComputedStyleUpdatesForNode" => {
                self.tracked_style_node = if request.params["nodeId"].is_null() {
                    None
                } else {
                    self.dom.handle("DOM.getAttributes", &request.params)?;

                    Some(
                        request.params["nodeId"]
                            .as_i64()
                            .and_then(|id| i32::try_from(id).ok())
                            .ok_or_else(|| Error::new(-32602, "nodeId required"))?,
                    )
                };

                Ok(json!({}))
            }
            "Overlay.highlightNode" | "Overlay.hideHighlight" | "Overlay.setInspectMode" => {
                let command = self.dom.overlay_command(method, &request.params)?;

                if !self.instance.info.element_interaction {
                    if matches!(
                        command,
                        Command::HighlightElement { id: None, .. }
                            | Command::InspectMode { enabled: false, .. }
                    ) {
                        return Ok(json!({}));
                    }

                    return Err(Error::new(
                        -32000,
                        "Runtime highlight/picker support is unavailable",
                    ));
                }

                self.control(command.clone())?;

                if matches!(
                    &command,
                    Command::HighlightElement { id: Some(_), .. }
                        | Command::InspectMode { enabled: true, .. }
                ) {
                    self.interaction_used = true;
                }

                self.domains.insert("Overlay".into());

                match command {
                    Command::HighlightElement { id, .. } => {
                        self.highlight = id.and_then(|id| self.dom.node_id(&id))
                    }
                    Command::InspectMode { enabled, .. } => {
                        self.picking = enabled;
                        self.last_hover = None;
                    }
                    _ => {}
                }

                Ok(json!({}))
            }
            "Overlay.enable" => {
                self.domains.insert("Overlay".into());

                Ok(json!({}))
            }
            "Overlay.disable" => {
                self.cleanup_overlay();
                self.domains.remove("Overlay");

                Ok(json!({}))
            }
            "DOM.pushNodesByBackendIdsToFrontend" => {
                let (result, events) = self.dom.push_nodes(&request.params)?;

                self.child_events(events)?;

                Ok(result)
            }
            "Runtime.getProperties" => {
                let id = request.params["objectId"]
                    .as_str()
                    .ok_or_else(|| Error::new(-32602, "objectId required"))?;

                if request.params["accessorPropertiesOnly"] == true {
                    return Ok(json!({"result":[]}));
                }

                if id.starts_with("rake-node-") {
                    let node = self.dom.object_node(id)?;
                    self.load_details(node)?;
                    let mut properties = self.dom.node_properties(id)?;
                    if let Ok(location) = self.dom.source(&json!(node)).cloned() {
                        let source = self.source_descriptor(&location);
                        if let Some(url) = source.get("url") {
                            properties["result"].as_array_mut().expect("node properties array").push(json!({"name":"rakeSourceURL","value":{"type":"string","value":url},"configurable":false,"enumerable":true}));
                        }
                        if let Some(location) = source.get("location") {
                            properties["result"].as_array_mut().expect("node properties array").push(json!({"name":"rakeSourceLocation","value":{"type":"string","value":location},"configurable":false,"enumerable":true}));
                        }
                    }
                    return Ok(properties);
                }

                self.objects.properties(id)
            }
            "Runtime.releaseObject" => {
                if let Some(id) = request.params["objectId"].as_str() {
                    self.objects.release(id);
                    self.dom.release_node_object(id);
                }

                Ok(json!({}))
            }
            "Runtime.releaseObjectGroup" => {
                if let Some(group) = request.params["objectGroup"].as_str() {
                    self.dom.release_node_group(group);
                }
                if let Some(group) = request.params["objectGroup"].as_str() {
                    self.objects.release_group(group);
                }
                Ok(json!({}))
            }
            "Runtime.discardConsoleEntries" | "Log.clear" => {
                self.clear_console();
                Ok(json!({}))
            }
            "Network.getResponseBody" | "Network.getRequestPostData" => {
                self.network_body(&request.params, method == "Network.getResponseBody")
            }
            "Network.searchInResponseBody" => {
                let body = self.network_body(&request.params, true)?;
                super::network::search(&body, &request.params)
            }
            "Network.setCacheDisabled" if request.params["cacheDisabled"] == false => Ok(json!({})),
            "Network.setBypassServiceWorker" if request.params["bypass"] == false => Ok(json!({})),
            "Inspector.enable" | "Runtime.runIfWaitingForDebugger" => Ok(json!({})),
            "Network.enable" | "Log.enable" | "Runtime.enable" => {
                let domain = method.split('.').next().unwrap_or_default();
                let fresh = self.domains.insert(domain.into());

                if domain == "Network" && fresh && self.network_paused {
                    self.skip_network_backlog()
                        .map_err(|error| Error::new(-32000, error.to_string()))?;
                    self.network_paused = false;
                }
                if domain == "Runtime" && fresh {
                    self.event(Event::new("Runtime.executionContextCreated", json!({"context":{
                        "id":1, "origin":"rake://runtime", "name":self.instance.application_id,
                        "uniqueId":self.instance.info.runtime_id.to_string(), "auxData":{"isDefault":true,"frameId":"rake"}}})));
                }

                Ok(json!({}))
            }
            "Network.disable" | "Log.disable" | "Runtime.disable" => {
                if method == "Network.disable" {
                    self.network_paused = true;
                }
                self.domains
                    .remove(method.split('.').next().unwrap_or_default());

                Ok(json!({}))
            }
            "Page.enable" => {
                self.domains.insert("Page".into());
                Ok(json!({}))
            }
            "Page.disable" => {
                self.domains.remove("Page");
                self.source_tree_loaded = false;
                Ok(json!({}))
            }
            "Page.getResourceTree" => {
                if self
                    .instance
                    .info
                    .queries
                    .iter()
                    .any(|q| q == "inspect_tree_page" || q == "inspect_tree")
                {
                    self.refresh_dom()?;
                }
                self.source_tree_loaded = true;
                Ok(
                    json!({"frameTree":{"frame":frame(&self.instance.info.runtime_id.to_string()),"resources":self.sources.resources(),"childFrames":self.sources.documents()}}),
                )
            }
            "Page.searchInResource" => {
                let result = self
                    .source_content(&request.params)
                    .and_then(|text| super::sources::search(&text, &request.params));
                if let Err(error) = &result {
                    self.warning(format!("Rust source search failed: {}", error.message));
                }
                result
            }
            "Page.getResourceContent" => {
                Ok(json!({"content":self.source_content(&request.params)?,"base64Encoded":false}))
            }
            "Network.loadNetworkResource" => {
                let text = self.source_content(&request.params)?;
                self.sources.stream(text)
            }
            "IO.read"
                if request.params["handle"]
                    .as_str()
                    .is_some_and(|h| h.starts_with("rake-source-")) =>
            {
                self.sources.read(&request.params)
            }
            "IO.close"
                if request.params["handle"]
                    .as_str()
                    .is_some_and(|h| h.starts_with("rake-source-")) =>
            {
                self.sources.close(&request.params);
                Ok(json!({}))
            }
            "Page.getFrameTree" => Ok(
                json!({"frameTree":{"frame":frame(&self.instance.info.runtime_id.to_string()),"childFrames":self.sources.frames()}}),
            ),
            _ => Err(Error::new(
                -32601,
                format!("Unsupported CDP method: {method}"),
            )),
        }
    }

    fn source_content(&self, params: &Value) -> std::result::Result<String, Error> {
        if params["url"] == "rake://runtime" {
            return Ok("Rake native runtime. Captured Rust files are read-only source documents; JavaScript execution and Rust debugging are unavailable here.\n".into());
        }
        if let (Some(base), Some(url)) = (&self.source_base, params["url"].as_str()) {
            let prefix = format!("{base}/{}/source/frame/", self.instance.info.runtime_id);
            if let Some(tail) = url.strip_prefix(&prefix) {
                return super::server::frame_source_content(&self.instance, tail)
                    .map_err(|error| Error::new(-32000, error.to_string()));
            }
        }
        self.sources.content(params)
    }

    fn skip_network_backlog(&mut self) -> Result<()> {
        if let Reply::Diagnostics(snapshot) = query(
            &self.instance,
            Query::Diagnostics {
                after: self.network_cursor,
                kind: None,
            },
            TIMEOUT,
        )? && let Some(record) = snapshot.records.last()
        {
            self.network_cursor = record.sequence;
        }
        Ok(())
    }

    fn network_body(&self, params: &Value, response: bool) -> std::result::Result<Value, Error> {
        let id = params["requestId"]
            .as_str()
            .ok_or_else(|| Error::new(-32602, "Request ID required"))?;
        if let Some(body) = self.network_bodies.get(id, response) {
            return super::network::body(body, response);
        }
        let id = super::network::local_id(id, &self.instance.info.runtime_id.to_string())?;
        let Reply::Data(body) = query(
            &self.instance,
            Query::HttpBody {
                request_id: id,
                response,
            },
            TIMEOUT,
        )
        .map_err(|error| Error::new(-32000, error.to_string()))?
        else {
            return Err(Error::new(-32000, "HTTP capture unavailable"));
        };
        super::network::body(body, response)
    }

    fn retain_network_bodies(&mut self, record: &rake_diagnostics::Record, id: &str) {
        let phase = record.fields["phase"].as_str().unwrap_or_default();
        if !matches!(
            phase,
            "completed" | "failed" | "timed_out" | "cancelled" | "abandoned"
        ) {
            return;
        }
        let Some(request_id) = record.fields["request_id"].as_u64() else {
            return;
        };
        let mut bodies = [Value::Null, Value::Null];
        for (index, body) in bodies.iter_mut().enumerate() {
            if let Ok(Reply::Data(value)) = query(
                &self.instance,
                Query::HttpBody {
                    request_id,
                    response: index == 1,
                },
                TIMEOUT,
            ) {
                *body = value;
            }
        }
        self.network_bodies.insert(id.into(), bodies);
    }

    fn source_link(&mut self, record: &rake_diagnostics::Record) -> Option<String> {
        if let Some(source) = &record.source {
            let site = rake_protocol::inspection::SourceLocation {
                file: source.file.clone(),
                line: source.line,
                column: 1,
            };
            if let Some(url) = self.sources.admit(&site) {
                self.publish_sources();
                return Some(url);
            }
        }
        if let (Some(base), Some(source)) = (&self.source_base, &record.source) {
            let filename = std::path::Path::new(&source.file)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("source.rs");
            let filename: String = filename
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            let url = format!(
                "{base}/{}/source/{}/{filename}",
                self.instance.info.runtime_id, record.sequence
            );
            self.sources.alias(url.clone(), record);
            Some(url)
        } else {
            None
        }
    }

    fn publish_sources(&mut self) {
        if self.domains.contains("Page")
            && self.source_tree_loaded
            && self.outbound.len() + 32 < QUEUE_LIMIT
            && self.outbound_bytes < 512 * 1024
        {
            for event in self.sources.events() {
                self.event(event);
            }
        }
    }

    fn source_descriptor(&mut self, location: &rake_protocol::inspection::SourceLocation) -> Value {
        let mut value = serde_json::to_value(location).expect("source location serialization");
        if let Some(url) = self.sources.admit(location) {
            value["url"] = json!(url);
            value["lineNumber"] = json!(location.line.saturating_sub(1));
            value["columnNumber"] = json!(
                self.sources
                    .column_number(location)
                    .unwrap_or(location.column.saturating_sub(1) as usize)
            );
            value["location"] = json!(format!(
                "{url}:{}:{}",
                location.line,
                value["columnNumber"].as_u64().unwrap_or(0) + 1
            ));
            self.publish_sources();
        }
        value
    }

    fn clear_console(&mut self) {
        self.objects.release_group("console");
        // Captured source aliases also belong to Network and stay independently bounded.
        self.discard_diagnostics = true;
        // Advance the per-connection cursor, without deleting application diagnostics.
        if let Ok(Reply::Diagnostics(snapshot)) = query(
            &self.instance,
            Query::Diagnostics {
                after: self.cursor,
                kind: None,
            },
            TIMEOUT,
        ) {
            self.accept_diagnostics(&snapshot);
            self.discard_diagnostics = false;
        }
        self.process_output.poll(unix_seconds(), true);
        self.outbound.retain(|(event, _)| {
            !matches!(
                event.method.as_str(),
                "Runtime.consoleAPICalled" | "Log.entryAdded"
            )
        });
        self.outbound_bytes = self.outbound.iter().map(|(_, bytes)| bytes).sum();
    }

    fn accept_diagnostics(&mut self, snapshot: &rake_diagnostics::Snapshot) {
        self.loss.observe(snapshot, self.cursor);
        if let Some(record) = snapshot.records.last() {
            self.cursor = record.sequence;
        }
    }

    fn poll_output(&mut self, include_mirrors: bool) {
        if self.domains.contains("Log") || self.domains.contains("Runtime") {
            for event in self.process_output.poll(unix_seconds(), include_mirrors) {
                self.event(event);
            }
        }
    }

    fn poll(&mut self) -> Result<()> {
        if self.domains.contains("DOMStorage") {
            match self.storage.poll(&self.instance) {
                Ok(events) => {
                    for event in events {
                        self.event(event);
                    }
                }
                Err(error) => {
                    self.storage.clear();
                    self.warning(format!(
                        "Storage refresh stopped: {}; refresh the storage table to retry",
                        error.message
                    ));
                }
            }
        }
        let live = query(&self.instance, Query::Info {}, TIMEOUT);
        // Raw stdout/stderr survive failed IPC; marked diagnostic mirrors are fallback only.
        self.poll_output(live.is_err());
        match live? {
            Reply::Info(info) if info.runtime_id == self.instance.info.runtime_id => {}
            _ => return Err("runtime identity changed".into()),
        }
        if self.memory.active
            && self
                .memory
                .renewed
                .is_none_or(|last| last.elapsed() >= Duration::from_secs(1))
        {
            if let Err(error) = self.control(self.memory.command(true)) {
                self.memory.active = false;
                self.memory.metadata["recording"] = json!(false);
                self.warning(format!("Native memory capture stopped: {}", error.message));
            } else {
                self.memory.renewed = Some(Instant::now());
            }
        }

        if self.domains.contains("DOM")
            || self.domains.contains("Accessibility")
            || self.domains.contains("Page")
        {
            self.refresh_dom().map_err(|e| e.message)?;
        }

        self.poll_overlay().map_err(|error| error.message)?;
        if self.frame_trace.active {
            let result = self
                .poll_frames(true)
                .and_then(|_| self.poll_performance_diagnostics());
            if let Err(error) = result {
                if error.message == "Recording lease expired"
                    || error.message.starts_with("Recording clock restarted")
                {
                    self.finish_trace(Some("capture became unavailable"));
                } else {
                    return Err(error.message.into());
                }
            }
        }

        self.publish_sources();
        let console = self.domains.contains("Log") || self.domains.contains("Runtime");
        let network = self.domains.contains("Network");
        if !console && !network {
            return Ok(());
        }
        let after = match (console, network) {
            (true, true) => self.cursor.min(self.network_cursor),
            (false, true) => self.network_cursor,
            _ => self.cursor,
        };
        let Reply::Diagnostics(snapshot) = query(
            &self.instance,
            Query::Diagnostics { after, kind: None },
            TIMEOUT,
        )?
        else {
            return Err("unexpected diagnostics reply".into());
        };

        let discard = std::mem::take(&mut self.discard_diagnostics);
        let warning = console
            .then(|| self.loss.observe(&snapshot, self.cursor))
            .flatten();

        if let Some(record) = snapshot.records.last() {
            self.epoch_seconds
                .get_or_insert_with(|| unix_seconds() - record.elapsed_us as f64 / 1e6);
        }

        if !discard && (self.domains.contains("Log") || self.domains.contains("Runtime")) {
            if let Some(warning) = warning {
                self.warning(warning);
            }

            for record in snapshot
                .records
                .iter()
                .filter(|record| record.sequence > after)
            {
                self.sources.observe(record);
                if record.sequence <= self.cursor || record.kind != "event" {
                    continue;
                }
                let mut event = if self.domains.contains("Log") {
                    self.objects
                        .logged(record, self.epoch_seconds.unwrap_or_default())
                } else {
                    self.objects
                        .called(record, self.epoch_seconds.unwrap_or_default())
                };
                if let Some(url) = self.source_link(record) {
                    if event.method == "Log.entryAdded" {
                        event.params["entry"]["url"] = json!(url);
                    } else {
                        event.params["stackTrace"]["callFrames"][0]["url"] = json!(url);
                    }
                }
                self.event(event);
            }
        }
        if console && let Some(record) = snapshot.records.last() {
            self.cursor = record.sequence;
        }

        if self.domains.contains("Network") {
            for record in &snapshot.records {
                if record.sequence <= self.network_cursor {
                    continue;
                }
                self.sources.observe(record);
                for mut event in super::network::translate(record) {
                    let runtime = self.instance.info.runtime_id.to_string();
                    let id = format!(
                        "{runtime}:{}",
                        event.params["requestId"].as_str().unwrap_or_default()
                    );
                    event.params["requestId"] = json!(id);
                    if event.params.get("loaderId").is_some() {
                        event.params["loaderId"] = json!(runtime);
                    }
                    self.retain_network_bodies(record, &id);
                    let epoch = self.epoch_seconds.unwrap_or_default();
                    let origin = *self.network_epoch.get_or_insert(epoch);
                    let offset = epoch - origin;
                    let timestamp = event.params["timestamp"].as_f64().unwrap_or_default();
                    event.params["timestamp"] = json!(timestamp + offset);
                    if let Some(time) = event.params["response"]["timing"]["requestTime"].as_f64() {
                        event.params["response"]["timing"]["requestTime"] = json!(time + offset);
                    }
                    if event.method == "Network.requestWillBeSent" {
                        if record.source.is_some()
                            && let Some(url) = self.source_link(record)
                        {
                            event.params["initiator"]["stack"]["callFrames"][0]["url"] = json!(url);
                        }
                        event.params["wallTime"] = json!(epoch + timestamp);
                    }

                    self.event(event);
                }
            }
        }

        if network && let Some(record) = snapshot.records.last() {
            self.network_cursor = record.sequence;
        }

        Ok(())
    }

    fn start_memory(&mut self, params: &Value) -> std::result::Result<(), Error> {
        if !self
            .instance
            .info
            .queries
            .iter()
            .any(|query| query == "memory")
        {
            return Err(Error::new(
                -32000,
                "Native memory capture requires devtools-memory and an installed TrackingAllocator",
            ));
        }
        let command = self.memory.begin(params)?;
        self.control(command)?;
        self.memory.active = true;
        self.memory.renewed = Some(Instant::now());
        Ok(())
    }

    fn memory_capture(
        &mut self,
        include_freed: bool,
    ) -> std::result::Result<rake_protocol::inspection::memory::Snapshot, Error> {
        let owner = self
            .memory
            .owner
            .as_ref()
            .ok_or_else(|| Error::new(-32000, "Start a native memory recording first"))?;
        let reply = query(
            &self.instance,
            Query::Memory {
                owner: owner.clone(),
                include_freed,
            },
            Duration::from_secs(30),
        )
        .map_err(|error| Error::new(-32000, error.to_string()))?;
        let Reply::Memory(capture) = reply else {
            return Err(Error::new(-32000, "Unexpected memory capture reply"));
        };
        if !capture.recording {
            self.memory.active = false;
        }
        Ok(capture)
    }

    fn memory_status(&self) -> Value {
        let mut value = if self.memory.metadata.is_object() {
            self.memory.metadata.clone()
        } else {
            json!({})
        };
        value["available"] = json!(
            self.instance
                .info
                .queries
                .iter()
                .any(|query| query == "memory")
        );
        value["recording"] = json!(self.memory.active);
        value
    }

    fn memory_profile(&mut self) -> std::result::Result<Value, Error> {
        let capture = self.memory_capture(self.memory.include_freed)?;
        let frames = self.memory_frames(&capture);
        let profile = self.memory.profile(&capture, &frames)?;
        if let Some(omitted) = self.memory.metadata["omitted"]
            .as_u64()
            .filter(|omitted| *omitted > 0)
        {
            self.warning(format!(
                "Native memory capture is partial: {} known omissions",
                omitted
            ));
        }
        Ok(profile)
    }

    fn memory_frames(
        &mut self,
        capture: &rake_protocol::inspection::memory::Snapshot,
    ) -> Vec<Value> {
        capture.sites.iter().map(|site| {
            let source = if let (Some(file),Some(line)) = (&site.file,site.line) {
                self.source_descriptor(&rake_protocol::inspection::SourceLocation { file: file.clone(), line, column: site.column.unwrap_or(1).max(1) })
            } else { json!({}) };
            json!({"functionName":site.function,"scriptId":"","url":source["url"].as_str().unwrap_or(""),
                "lineNumber":source["lineNumber"].as_i64().unwrap_or(-1),"columnNumber":source["columnNumber"].as_i64().unwrap_or(-1)})
        }).collect()
    }

    fn stop_memory(&mut self) {
        // A start may have reached the runtime even if its acknowledgement was lost.
        if self.memory.owner.is_some() {
            let _ = self.control(self.memory.command(false));
            self.memory.active = false;
            self.memory.metadata["recording"] = json!(false);
        }
    }

    fn control(&self, command: Command) -> std::result::Result<(), Error> {
        let reply = crate::terminal::exchange(
            &self.instance,
            &RuntimeRequest {
                protocol: PROTOCOL_VERSION,
                runtime_id: self.instance.info.runtime_id,
                key: None,
                message: RuntimeMessage::Command(command),
            },
            TIMEOUT,
        )
        .map_err(|e| Error::new(-32000, e.to_string()))?;

        if !matches!(reply, Reply::Acknowledged) {
            return Err(Error::new(-32000, "Unexpected inspection control reply"));
        }

        Ok(())
    }

    fn cleanup(&mut self) {
        self.stop_memory();
        if self.frame_trace.active {
            let _ = self.control(Command::RecordFrames {
                enabled: false,
                lease_ms: 0,
            });
            self.frame_trace.active = false;
        }
        self.cleanup_overlay();
    }

    fn cleanup_overlay(&mut self) {
        if self.interaction_used {
            let _ = self.control(Command::InspectMode {
                enabled: false,
                lease_ms: 0,
            });

            let _ = self.control(Command::HighlightElement {
                id: None,
                generation: String::new(),
            });
        }

        self.interaction_used = false;
        self.picking = false;
        self.highlight = None;
    }

    fn poll_overlay(&mut self) -> std::result::Result<(), Error> {
        if self.picking {
            let Reply::InspectPick(pick) = query(&self.instance, Query::InspectPick {}, TIMEOUT)
                .map_err(|e| Error::new(-32000, e.to_string()))?
            else {
                return Err(Error::new(-32000, "Unexpected picker reply"));
            };

            if (pick.hovered.is_some() || pick.picked.is_some())
                && self.dom.generation() != Some(pick.generation.as_str())
            {
                self.refresh_dom()?;
            }

            if pick.mode {
                self.control(Command::InspectMode {
                    enabled: true,
                    lease_ms: 1000,
                })?;
            } else {
                self.picking = false;
            }

            if let Some(id) = &pick.hovered {
                let hover = (pick.generation.clone(), id.clone());

                if self.last_hover.as_ref() != Some(&hover)
                    && let Some(node) = self.dom.node_id(id)
                    && let Some(events) = self.dom.reveal_node(node)?
                {
                    self.child_events(events)?;

                    self.event(Event::new(
                        "Overlay.nodeHighlightRequested",
                        json!({"nodeId":node}),
                    ));
                    self.last_hover = Some(hover);
                }
            } else {
                self.last_hover = None;
            }

            if pick.pick_sequence > self.last_pick_sequence
                && let Some(id) = pick.picked.as_deref()
            {
                if let Some(node) = self.dom.node_id(id) {
                    self.event(Event::new(
                        "Overlay.inspectNodeRequested",
                        json!({"backendNodeId":node}),
                    ));
                }

                self.last_pick_sequence = pick.pick_sequence;
            }
        }

        if let Some(id) = self.highlight
            && self
                .dom
                .overlay_command("Overlay.highlightNode", &json!({"nodeId":id}))
                .and_then(|command| self.control(command))
                .is_err()
        {
            self.highlight = None;
        }

        Ok(())
    }

    fn flush(&mut self, socket: &mut WebSocket<TcpStream>) -> Result<()> {
        while let Some((event, bytes)) = self.outbound.pop_front() {
            self.outbound_bytes -= bytes;
            socket.send(Message::text(serde_json::to_string(&event)?))?;
        }

        Ok(())
    }

    fn child_events(&mut self, events: Vec<Value>) -> std::result::Result<(), Error> {
        let events = events
            .into_iter()
            .map(|params| Event::new("DOM.setChildNodes", params))
            .collect::<Vec<_>>();
        let bytes = events
            .iter()
            .try_fold(0usize, |sum, event| {
                serde_json::to_vec(event)
                    .ok()
                    .and_then(|bytes| sum.checked_add(bytes.len()))
            })
            .unwrap_or(usize::MAX);
        if self.outbound.len() + events.len() > QUEUE_LIMIT
            || self.outbound_bytes.saturating_add(bytes) > 1024 * 1024
        {
            self.dropped = self.dropped.saturating_add(self.outbound.len() as u64);
            self.outbound.clear();
            self.outbound_bytes = 0;
            self.sources.republish();
            self.dom.reset_delivery();
            self.tracked_style_node = None;
            self.event(Event::new("DOM.documentUpdated", json!({})));
            return Err(Error::new(
                -32000,
                "Tree delivery exceeded the queue budget; request the document and repeat navigation",
            ));
        }
        for event in events {
            self.event(event);
        }
        Ok(())
    }

    fn refresh_dom(&mut self) -> std::result::Result<(), Error> {
        let paged = self
            .instance
            .info
            .queries
            .iter()
            .any(|q| q == "inspect_tree_page");
        let snapshot = if paged {
            let mut generation = None;
            let mut offset = 0;
            let mut combined: Option<rake_protocol::inspection::Snapshot> = None;
            loop {
                let reply = query(
                    &self.instance,
                    Query::InspectTreePage {
                        generation: generation.clone(),
                        offset,
                    },
                    TIMEOUT,
                )
                .map_err(|e| Error::new(-32000, e.to_string()))?;
                let Reply::ElementPage(page) = reply else {
                    return Err(Error::new(-32000, "Unexpected tree page reply"));
                };
                if page.total > rake_protocol::inspection::MAX_ELEMENTS
                    || page.snapshot.elements.len() > rake_protocol::inspection::TREE_PAGE_SIZE
                    || generation
                        .as_ref()
                        .is_some_and(|id| id != &page.snapshot.capture)
                {
                    return Err(Error::new(-32000, "Invalid tree page"));
                }
                generation = Some(page.snapshot.capture.clone());
                if let Some(snapshot) = &mut combined {
                    snapshot.elements.extend(page.snapshot.elements);
                } else {
                    combined = Some(page.snapshot);
                }
                match page.next {
                    Some(next) if next > offset && next <= page.total => offset = next,
                    Some(_) => return Err(Error::new(-32000, "Invalid tree page cursor")),
                    None => break,
                }
            }
            combined.expect("first page always supplies metadata")
        } else {
            let reply = query(&self.instance, Query::InspectTree {}, TIMEOUT)
                .map_err(|e| Error::new(-32000, e.to_string()))?;
            let Reply::Elements(snapshot) = reply else {
                return Err(Error::new(-32000, "Unexpected inspection reply"));
            };
            snapshot
        };

        for element in &snapshot.elements {
            if let Some(source) = &element.source {
                self.sources.admit(source);
            }
        }
        self.publish_sources();
        let mut reset = if paged {
            self.dom.update_index(snapshot)?
        } else {
            self.dom.update(snapshot)?
        };
        let updates = self.dom.take_updates();
        let bytes = updates
            .iter()
            .try_fold(0usize, |sum, event| {
                serde_json::to_vec(event)
                    .ok()
                    .and_then(|bytes| sum.checked_add(bytes.len()))
            })
            .unwrap_or(usize::MAX);

        // A partial mutation batch would leave Chrome permanently out of sync.
        if !updates.is_empty()
            && (self.outbound.len() + updates.len() + 3 > QUEUE_LIMIT
                || self
                    .outbound_bytes
                    .saturating_add(bytes)
                    .saturating_add(1024)
                    > 1024 * 1024)
        {
            self.dropped = self.dropped.saturating_add(self.outbound.len() as u64);
            self.outbound.clear();
            self.outbound_bytes = 0;
            self.dom.reset_delivery();
            reset = true;
        }

        if reset && self.domains.contains("DOM") {
            self.tracked_style_node = None;
            self.event(Event::new("DOM.documentUpdated", json!({})));
        } else if self.domains.contains("DOM") {
            for event in updates {
                self.event(event);
            }
        }

        if let Some(node) = self.tracked_style_node
            && self.dom.control_id(node).is_ok()
        {
            self.load_details(node)?;
        }
        let accessibility = self.dom.take_accessibility_updates();
        if self.domains.contains("Accessibility") {
            let bytes = serde_json::to_vec(&accessibility).map_or(usize::MAX, |value| value.len());
            if reset || bytes > 256 * 1024 {
                if let Ok(root) = self
                    .dom
                    .accessibility("Accessibility.getRootAXNode", &json!({}))
                {
                    self.event(Event::new(
                        "Accessibility.loadComplete",
                        json!({"root":root["node"]}),
                    ));
                }
            } else if !accessibility.is_empty() {
                self.event(Event::new(
                    "Accessibility.nodesUpdated",
                    json!({"nodes":accessibility}),
                ));
            }
        }
        let changed = self.dom.take_style_updates();

        if !changed.is_empty() && self.domains.contains("CSS") {
            self.event(Event::new(
                "CSS.styleSheetChanged",
                json!({"styleSheetId":"rake-resolved"}),
            ));

            for id in changed {
                if self.tracked_style_node == Some(id) {
                    self.event(Event::new("CSS.computedStyleUpdated", json!({"nodeId":id})));
                }
            }
        }

        Ok(())
    }

    fn finish_trace(&mut self, reason: Option<&str>) {
        if !self.frame_trace.active {
            return;
        }
        self.frame_trace.loss |= reason.is_some();
        if self
            .control(Command::RecordFrames {
                enabled: false,
                lease_ms: 0,
            })
            .is_err()
        {
            self.frame_trace.loss = true;
        } else {
            for _ in 0..32 {
                match self.poll_frames(false) {
                    Ok(false) => break,
                    Ok(true) => {}
                    Err(_) => {
                        self.frame_trace.loss = true;
                        break;
                    }
                }
            }
        }
        if self.poll_performance_diagnostics().is_err() {
            self.frame_trace.loss = true;
        }
        let mut complete = self.frame_trace.end();
        let report = self.frame_trace.report();
        self.trace_events(report);
        complete["dataLossOccurred"] = json!(self.frame_trace.loss);
        if reason.is_some() && !self.frame_trace.has_stream() {
            // Chrome's ReportEvents client must remain active until the user presses Stop.
            self.pending_trace_complete = Some(complete);
        } else {
            self.event(Event::new("Tracing.tracingComplete", complete));
        }
    }

    fn trace_events(&mut self, events: Vec<Value>) {
        for chunk in events.chunks(128) {
            let before = self.dropped;
            self.event(Event::new("Tracing.dataCollected", json!({"value":chunk})));
            self.frame_trace.loss |= self.dropped != before;
        }
    }

    fn poll_performance_diagnostics(&mut self) -> std::result::Result<(), Error> {
        if !self.frame_trace.causal.enabled() {
            return Ok(());
        }
        let Reply::Diagnostics(snapshot) = query(
            &self.instance,
            Query::Diagnostics {
                after: self.frame_trace.causal.cursor,
                kind: None,
            },
            TIMEOUT,
        )
        .map_err(|error| Error::new(-32000, error.to_string()))?
        else {
            return Err(Error::new(-32000, "Unexpected performance diagnostics"));
        };
        let mut sources = std::collections::BTreeMap::new();
        for record in &snapshot.records {
            if record.source.is_some()
                && let Some(url) = self.source_link(record)
            {
                sources.insert(record.sequence, url);
            }
        }
        let events = self.frame_trace.diagnostics(&snapshot, &sources);
        self.trace_events(events);
        Ok(())
    }

    fn poll_frames(&mut self, renew: bool) -> std::result::Result<bool, Error> {
        if renew {
            self.control(Command::RecordFrames {
                enabled: true,
                lease_ms: 5000,
            })?;
        }
        let Reply::Frames(frames) = query(
            &self.instance,
            Query::Frames {
                after: self.frame_trace.cursor,
            },
            TIMEOUT,
        )
        .map_err(|error| Error::new(-32000, error.to_string()))?
        else {
            return Err(Error::new(-32000, "Unexpected frames reply"));
        };
        if renew && !frames.recording {
            return Err(Error::new(-32000, "Recording lease expired"));
        }
        if frames
            .clock
            .as_ref()
            .is_some_and(|clock| !self.frame_trace.causal.consistent(clock))
        {
            return Err(Error::new(
                -32000,
                "Recording clock restarted; ending partial capture",
            ));
        }
        for frame in &frames.frames {
            for (index, node) in frame.nodes.iter().enumerate() {
                let url = super::frames::source_url(
                    self.frame_trace.source_prefix.as_deref(),
                    frame.sequence,
                    index,
                    node,
                );
                self.sources.capture(url, node.source.clone());
                if let Some(url) = self.sources.admit(&node.source) {
                    self.frame_trace
                        .source_urls
                        .insert(node.source.file.clone(), url);
                }
            }
        }
        let events = self.frame_trace.collect(&frames);
        self.trace_events(events);
        Ok(frames.more)
    }

    fn source_location(
        &mut self,
        selector: &Value,
    ) -> std::result::Result<rake_protocol::inspection::SourceLocation, Error> {
        if selector.get("url").is_some() {
            return self.sources.site(selector);
        }
        self.refresh_dom()?;
        let node = self.dom.source_node(selector)?;
        self.load_details(node)?;
        self.dom.source(selector).cloned()
    }

    fn source_selector(&self, argument: &str) -> std::result::Result<Value, Error> {
        if let Some(rest) = argument.trim().strip_prefix("$0") {
            let node = self.dom.inspected_node()?;
            if rest.trim().is_empty() {
                return Ok(json!(node));
            }
            let target = rest
                .trim()
                .strip_prefix(',')
                .and_then(|text| serde_json::from_str::<Value>(text).ok())
                .ok_or_else(|| Error::new(-32602, "Expected a JSON source target after $0"))?;
            return Ok(json!({"nodeId":node,"target":target}));
        }
        serde_json::from_str(argument).map_err(|_| {
            Error::new(
                -32602,
                "openSource requires $0, a JSON node/semantic ID, or {nodeId,target}",
            )
        })
    }

    fn load_details(&mut self, node: i32) -> std::result::Result<(), Error> {
        if !self
            .instance
            .info
            .queries
            .iter()
            .any(|q| q == "inspect_tree_page")
        {
            return Ok(());
        }
        let id = self.dom.control_id(node)?.to_owned();
        let Reply::Element(element) = query(&self.instance, Query::InspectElement { id }, TIMEOUT)
            .map_err(|e| Error::new(-32000, e.to_string()))?
        else {
            return Err(Error::new(-32000, "Unexpected detail reply"));
        };
        if let Some(source) = &element.source {
            self.sources.admit(source);
        }
        if let Some(view) = &element.view {
            for style in &view.style_sources {
                self.sources.admit(&style.source);
            }
        }
        for handler in &element.handlers {
            self.sources.admit(&handler.source);
        }
        self.publish_sources();
        self.dom.details(node, element)
    }

    fn replace(&mut self, instance: Instance) {
        self.finish_trace(Some("runtime replaced"));
        self.cleanup();
        self.memory = super::memory::Memory::default();
        self.last_hover = None;
        self.last_pick_sequence = 0;
        self.tracked_style_node = None;
        let old = self.instance.info.runtime_id;
        let old_sources = self.sources.frames();

        self.instance = instance;
        self.cursor = 0;
        self.network_cursor = 0;
        self.epoch_seconds = None;
        self.objects.clear();
        self.sources.clear();
        if let Some(base) = &self.source_base {
            self.sources
                .bind(base, &self.instance.info.runtime_id.to_string());
        }
        self.discard_diagnostics = false;
        self.loss = super::console::Loss::default();
        self.dom.clear();
        self.edit_sheets.clear();
        self.outbound
            .retain(|(event, _)| event.method.starts_with("Tracing."));
        self.outbound_bytes = self.outbound.iter().map(|(_, bytes)| bytes).sum();
        self.dropped = 0;
        self.event(Event::new("HeapProfiler.resetProfiles", json!({})));
        if self.domains.contains("DOMStorage") {
            for event in self.storage.reset() {
                self.event(event);
            }
        }
        if self.domains.contains("Page") && self.source_tree_loaded {
            for document in old_sources {
                self.event(Event::new(
                    "Page.frameDetached",
                    json!({"frameId":document["frame"]["id"],"reason":"remove"}),
                ));
            }
        }
        self.event(Event::new("Page.frameNavigated", json!({"frame": {"id":"rake","loaderId":self.instance.info.runtime_id.to_string(),
            "url":"rake://runtime","domainAndRegistry":"","securityOrigin":"rake://runtime","mimeType":"text/plain",
            "secureContextType":"Secure","crossOriginIsolatedContextType":"NotIsolated","gatedAPIFeatures":[]}, "type":"Navigation"})));
        self.event(Event::new("Runtime.executionContextsCleared", json!({})));
        self.event(Event::new("DOM.documentUpdated", json!({})));
        self.event(super::console::entry(
            "info",
            format!("restored from {old}"),
            unix_seconds() * 1000.,
            "rake://adapter",
        ));

        if self.domains.contains("Runtime") {
            self.event(Event::new("Runtime.executionContextCreated",json!({"context":{"id":1,"origin":"rake://runtime","name":self.instance.application_id,
                "uniqueId":self.instance.info.runtime_id.to_string(),"auxData":{"isDefault":true,"frameId":"rake"}}})));
        }
    }

    fn successor(&self, directory: &std::path::Path) -> Option<Instance> {
        let instances = rake_client::local::discover(directory).ok()?;

        let mut successors=instances.into_iter().filter(|i|i.application_id==self.instance.application_id && i.info.runtime_id!=self.instance.info.runtime_id)
            .filter(|i|matches!(query(i,Query::Reload {},TIMEOUT),Ok(Reply::Reload(status)) if status.restored_from==Some(self.instance.info.runtime_id)));

        let successor = successors.next()?;

        if successors.next().is_some() {
            return None;
        }

        Some(successor)
    }

    fn warning(&mut self, text: String) {
        if self.domains.contains("Log") {
            self.event(super::console::entry(
                "warning",
                text,
                unix_seconds() * 1000.,
                "rake://adapter",
            ));
        } else if self.domains.contains("Runtime") {
            self.event(Event::new("Runtime.consoleAPICalled",json!({"type":"warning","args":[{"type":"string","value":text}],"executionContextId":1,"timestamp":unix_seconds()*1000.})));
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.cleanup();
    }
}

fn edit_node(value: &Value) -> std::result::Result<i32, Error> {
    value
        .as_str()
        .and_then(|id| id.strip_prefix("rake-edit-"))
        .and_then(|id| id.parse().ok())
        .ok_or_else(|| Error::new(-32602, "Only DevTools override sheets are editable"))
}

fn unix_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn frame(runtime: &str) -> Value {
    json!({"id":"rake", "loaderId":runtime, "url":"rake://runtime", "domainAndRegistry":"",
        "securityOrigin":"rake://runtime", "mimeType":"text/plain", "secureContextType":"Secure",
        "crossOriginIsolatedContextType":"NotIsolated", "gatedAPIFeatures":[]})
}

pub(super) fn run(
    mut socket: WebSocket<TcpStream>,
    instance: Instance,
    output: Option<std::path::PathBuf>,
    directory: std::path::PathBuf,
    source_base: String,
    access: &super::sources::Access,
) -> Result<()> {
    socket
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(20)))?;
    socket
        .get_mut()
        .set_write_timeout(Some(Duration::from_secs(1)))?;
    let mut session = Session::new(instance, output);
    session
        .sources
        .bind(&source_base, &session.instance.info.runtime_id.to_string());
    access
        .register(&session.sources)
        .map_err(|error| error.message)?;
    session.source_base = Some(source_base);
    let mut next_poll = Instant::now();
    let mut missing_since: Option<Instant> = None;

    loop {
        match socket.read() {
            Ok(Message::Text(text)) => {
                let response = match serde_json::from_str::<Request>(&text) {
                    Ok(request) => Response::new(Some(request.id), session.handle(&request)),
                    Err(error) => Response::new(None, Err(Error::new(-32600, error.to_string()))),
                };

                session.flush(&mut socket)?;
                socket.send(Message::text(response.encode()?))?;
            }
            Ok(Message::Close(_)) | Err(tungstenite::Error::ConnectionClosed) => return Ok(()),
            Ok(_) => {}
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => return Err(error.into()),
        }

        if session.detached.is_none()
            && (!session.domains.is_empty() || session.frame_trace.active)
            && Instant::now() >= next_poll
        {
            match session.poll() {
                Ok(()) => missing_since = None,
                Err(error) => {
                    if let Some(successor) = session.successor(&directory) {
                        session.replace(successor);
                        missing_since = None;
                    } else if missing_since.get_or_insert_with(Instant::now).elapsed()
                        >= Duration::from_secs(5)
                    {
                        if session.frame_trace.active || session.pending_trace_complete.is_some() {
                            session.finish_trace(Some("runtime disconnected"));
                            session.detached = Some(error.to_string());
                            session.flush(&mut socket)?;
                            continue;
                        }
                        session.poll_output(true);
                        let text = format!(
                            "Runtime disconnected: {error}. Retained supervisor output is shown above."
                        );
                        if session.domains.contains("Log") {
                            session.event(super::console::entry(
                                "error",
                                text,
                                unix_seconds() * 1000.,
                                "rake://adapter",
                            ));
                        } else if session.domains.contains("Runtime") {
                            session.event(Event::new("Runtime.consoleAPICalled",json!({"type":"error","args":[{"type":"string","value":text}],"executionContextId":1,"timestamp":unix_seconds()*1000.})));
                        }
                        session.flush(&mut socket)?;
                        socket.send(Message::text(serde_json::to_string(&Event::new(
                            "Inspector.detached",
                            json!({"reason":error.to_string()}),
                        ))?))?;
                        let _ = socket.close(None);

                        return Ok(());
                    }
                }
            }

            next_poll = Instant::now()
                + if session.picking || session.highlight.is_some() {
                    Duration::from_millis(100)
                } else {
                    INTERVAL
                };
        }

        session.flush(&mut socket)?;
        if let Some(reason) = &session.detached
            && !session.frame_trace.has_stream()
            && session.pending_trace_complete.is_none()
        {
            socket.send(Message::text(serde_json::to_string(&Event::new(
                "Inspector.detached",
                json!({"reason":reason}),
            ))?))?;
            return Ok(());
        }

        if session.dropped > 0
            && (session.domains.contains("Log") || session.domains.contains("Runtime"))
        {
            let dropped = std::mem::take(&mut session.dropped);

            session.warning(format!("Rake CDP outbound queue dropped {dropped} events"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_snapshots_are_atomic_and_console_preview_never_stops_capture() {
        let (instance, worker, path) = fixture(
            |request| match request.message {
                RuntimeMessage::Command(Command::RecordMemory { .. }) => Reply::Acknowledged,
                RuntimeMessage::Query(Query::Memory { .. }) => {
                    Reply::Memory(rake_protocol::inspection::memory::Snapshot {
                        generation: 1,
                        recording: true,
                        interval: 32768,
                        omitted: 0,
                        freed_omitted: 0,
                        sites: vec![rake_protocol::inspection::memory::Site {
                            address: "0x42".into(),
                            function: "probe🙂".into(),
                            file: None,
                            line: None,
                            column: None,
                        }],
                        samples: vec![rake_protocol::inspection::memory::Sample {
                            ordinal: 1,
                            size: 1024,
                            estimated_bytes: 32768,
                            live: true,
                            stack: vec![0],
                        }],
                    })
                }
                _ => panic!("unexpected memory fixture command"),
            },
            4,
        );
        let mut session = Session::new(instance, None);
        session.instance.info.queries.push("memory".into());
        let call = |method: &str, params: Value| Request {
            id: 1,
            method: method.into(),
            params,
        };
        session
            .handle(&call("HeapProfiler.startSampling", json!({})))
            .unwrap();
        session.handle(&call("Overlay.disable", json!({}))).unwrap();
        assert!(
            session.memory.active,
            "Chrome suspends Overlay around snapshots without ending memory tracking"
        );
        session
            .handle(&call(
                "Runtime.compileScript",
                json!({"expression":"rake.stopMemory()"}),
            ))
            .unwrap();
        assert!(
            session
                .handle(&call(
                    "Runtime.evaluate",
                    json!({"expression":"rake.stopMemory()","throwOnSideEffect":true})
                ))
                .is_err()
        );
        assert!(session.memory.active);
        for _ in 0..QUEUE_LIMIT {
            session.event(Event::new("occupied", json!({})));
        }
        assert!(
            session
                .handle(&call(
                    "HeapProfiler.takeHeapSnapshot",
                    json!({"reportProgress":true})
                ))
                .is_err()
        );
        assert!(
            !session
                .outbound
                .iter()
                .any(|(event, _)| event.method.starts_with("HeapProfiler."))
        );
        session.outbound.clear();
        session.outbound_bytes = 0;
        session
            .handle(&call(
                "HeapProfiler.takeHeapSnapshot",
                json!({"reportProgress":true}),
            ))
            .unwrap();
        assert_eq!(
            session.outbound[0].0.method,
            "HeapProfiler.reportHeapSnapshotProgress"
        );
        let text: String = session
            .outbound
            .iter()
            .filter(|(event, _)| event.method == "HeapProfiler.addHeapSnapshotChunk")
            .map(|(event, _)| event.params["chunk"].as_str().unwrap())
            .collect();
        let snapshot: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(snapshot["snapshot"]["node_count"], 2);
        session.handle(&call("Rake.stopMemory", json!({}))).unwrap();
        assert_eq!(session.memory_status()["recording"], false);
        worker.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn clearing_advances_only_this_connection_and_replacement_expires_handles() {
        let info = rake::router::Router::<rake_protocol::router::NoRoute>::new(Default::default())
            .client()
            .info();
        let (instance, worker, path) = fixture(
            move |request| {
                match request.message {
            RuntimeMessage::Query(Query::Info {}) => {let mut info=info.clone();info.runtime_id=request.runtime_id;Reply::Info(info)},
            RuntimeMessage::Query(Query::Diagnostics {after,..}) => Reply::Diagnostics(rake_diagnostics::Snapshot {
                records:vec![serde_json::from_value(json!({"sequence":after+1,"elapsed_us":0,"level":"INFO","target":"test","kind":"event","spans":[],"fields":{"message":format!("event-{}",after+1)}})).unwrap()],
                dropped:0,output_dropped:0,evicted:0 }),
            _=>panic!("unexpected request"),
        }
            },
            5,
        );
        let mut session = Session::new(instance, None);
        session.domains.insert("Runtime".into());
        session.poll().unwrap();
        let old = session.outbound[0].0.params["args"][1]["objectId"]
            .as_str()
            .unwrap()
            .to_owned();
        session
            .handle(&Request {
                id: 1,
                method: "Runtime.discardConsoleEntries".into(),
                params: json!({}),
            })
            .unwrap();
        assert!(session.outbound.is_empty());
        assert_eq!(session.cursor, 2);
        assert!(session.objects.properties(&old).is_err());
        session.poll().unwrap();
        assert_eq!(session.outbound.len(), 1);
        assert_eq!(session.outbound[0].0.params["args"][0]["value"], "event-3");
        let retained = session.outbound[0].0.params["args"][1]["objectId"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut successor = session.instance.clone();
        successor.info.runtime_id = rake_protocol::router::RuntimeId(uuid::Uuid::new_v4());
        session.replace(successor);
        assert!(session.objects.properties(&retained).is_err());
        assert!(
            session
                .outbound
                .iter()
                .any(|(event, _)| event.method == "Page.frameNavigated")
        );
        assert!(
            session
                .outbound
                .iter()
                .any(|(event, _)| event.method == "Runtime.executionContextsCleared")
        );
        worker.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn standalone_console_has_discoverable_helpers_without_application_commands() {
        let router =
            rake::router::Router::<rake_protocol::router::NoRoute>::new(Default::default());
        let mut info = router.client().info();
        info.queries.retain(|q| q != "commands");
        let instance = Instance {
            application_id: "standalone".into(),
            pid: 0,
            info,
            endpoint: rake_client::local::Endpoint::Local {
                path: "/never-connect".into(),
            },
        };
        let mut session = Session::new(instance, None);
        let call = |expression| Request {
            id: 1,
            method: "Runtime.evaluate".into(),
            params: json!({"expression":expression,"throwOnSideEffect":true,"returnByValue":true}),
        };
        let help = session.handle(&call("rake.help()")).unwrap();
        assert_eq!(help["result"]["value"]["undoText"], "rake.undoText($0)");
        let commands = session.handle(&call("rake.commands()")).unwrap();
        assert_eq!(commands["result"]["value"], json!([]));
        assert!(
            session.handle(&call("rake.resetText($0)")).is_err(),
            "eager mutation must remain disabled"
        );
    }

    #[test]
    fn evaluation_errors_are_visible_exceptions_and_crash_output_survives_dead_ipc() {
        use std::os::unix::fs::OpenOptionsExt;
        let path = std::env::temp_dir().join(format!("rake-crash-{}.json", uuid::Uuid::new_v4()));
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        serde_json::to_writer(file, &json!({"records":[{"sequence":1,"elapsed_us":0,"stream":"stderr",
            "text":"thread 'main' panicked at app.rs:42:1: fixture panic\nstack backtrace:"}],"dropped":0})).unwrap();
        let router =
            rake::router::Router::<rake_protocol::router::NoRoute>::new(Default::default());
        let instance = Instance {
            application_id: "crash".into(),
            pid: 0,
            info: router.client().info(),
            endpoint: rake_client::local::Endpoint::Local {
                path: path.with_extension("missing.sock"),
            },
        };
        let mut session = Session::new(instance, Some(path.clone()));
        session.domains.insert("Log".into());
        assert!(session.poll().is_err());
        assert_eq!(session.outbound.len(), 1);
        assert_eq!(session.outbound[0].0.params["entry"]["level"], "error");
        assert!(
            session.outbound[0].0.params["entry"]["text"]
                .as_str()
                .unwrap()
                .contains("stack backtrace")
        );
        let error = session
            .handle(&Request {
                id: 1,
                method: "Runtime.evaluate".into(),
                params: json!({"expression":"1+1"}),
            })
            .unwrap();
        assert_eq!(error["result"]["subtype"], "error");
        assert!(
            error["result"]["description"]
                .as_str()
                .unwrap()
                .contains("registered command")
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn structured_logs_have_one_delivery_source_and_expandable_context() {
        let record: rake_diagnostics::Record =
            serde_json::from_value(json!({"sequence":1,"elapsed_us":0,
            "level":"WARN","target":"app::export","kind":"event","spans":["export"],
            "source":{"file":"app.rs","line":42},"fields":{"message":"export started","rows":3}}))
            .unwrap();
        let info = rake::router::Router::<rake_protocol::router::NoRoute>::new(Default::default())
            .client()
            .info();
        let (instance, worker, path) = fixture(
            move |request| match request.message {
                RuntimeMessage::Query(Query::Info {}) => {
                    let mut info = info.clone();
                    info.runtime_id = request.runtime_id;
                    Reply::Info(info)
                }
                RuntimeMessage::Query(Query::Diagnostics { after, .. }) => {
                    Reply::Diagnostics(rake_diagnostics::Snapshot {
                        records: if after == 0 {
                            vec![record.clone()]
                        } else {
                            vec![]
                        },
                        dropped: 0,
                        output_dropped: 0,
                        evicted: 0,
                    })
                }
                _ => panic!("unexpected request"),
            },
            4,
        );
        let mut session = Session::new(instance, None);
        session.domains.extend(["Log".into(), "Runtime".into()]);
        session.poll().unwrap();
        session.poll().unwrap();
        assert_eq!(
            session.outbound.len(),
            1,
            "Log and Runtime must not duplicate diagnostics"
        );
        let event = &session.outbound[0].0;
        assert_eq!(event.method, "Log.entryAdded");
        assert_eq!(event.params["entry"]["level"], "warning");
        assert_eq!(event.params["entry"]["lineNumber"], 41);
        let id = event.params["entry"]["args"][0]["objectId"]
            .as_str()
            .unwrap()
            .to_owned();
        let props = session.objects.properties(&id).unwrap();
        assert!(
            props["result"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["name"] == "spans")
        );
        assert_eq!(
            session.sources.resources()[0]["url"],
            "rake://source/app.rs"
        );
        worker.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn outbound_retention_is_bounded_by_count_and_bytes() {
        let router = rake::router::Router::<rake_protocol::router::NoRoute>::new(
            rake::router::RouterConfig::default(),
        );

        let instance = Instance {
            application_id: "test.queue".into(),
            pid: 1,
            info: router.client().info(),
            endpoint: rake_client::local::Endpoint::Local {
                path: "/unused".into(),
            },
        };

        let mut session = Session::new(instance, None);

        for _ in 0..QUEUE_LIMIT + 2 {
            session.event(Event::new("Log.entryAdded", json!({})));
        }

        assert_eq!(session.outbound.len(), QUEUE_LIMIT);
        assert_eq!(session.dropped, 2);
        session.outbound.clear();
        session.outbound_bytes = 0;
        session.event(Event::new(
            "Log.entryAdded",
            json!({"text":"x".repeat(1024*1024)}),
        ));
        assert!(session.outbound.is_empty());
        assert_eq!(session.dropped, 3);

        for _ in 0..QUEUE_LIMIT {
            session.event(Event::new("Log.entryAdded", json!({})));
        }
        assert!(
            session
                .child_events(vec![json!({"parentId":1,"nodes":[]})])
                .is_err()
        );
        assert_eq!(session.outbound.len(), 1);
        assert_eq!(
            session.outbound[0].0.method, "DOM.documentUpdated",
            "ancestor paths must never be partially delivered"
        );
    }

    fn fixture(
        mut reply: impl FnMut(RuntimeRequest) -> Reply + Send + 'static,
        requests: usize,
    ) -> (Instance, std::thread::JoinHandle<()>, std::path::PathBuf) {
        use std::io::{BufRead, BufReader, Write};
        let path =
            std::path::PathBuf::from(format!("/tmp/rake-overlay-{}.sock", uuid::Uuid::new_v4()));

        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();

        let router = rake::router::Router::<rake_protocol::router::NoRoute>::new(
            rake::router::RouterConfig::default(),
        );

        let mut info = router.client().info();

        info.element_interaction = true;
        info.queries.retain(|query| query != "inspect_tree_page");
        let instance = Instance {
            application_id: "test.overlay".into(),
            pid: 1,
            info,
            endpoint: rake_client::local::Endpoint::Local { path: path.clone() },
        };

        let worker = std::thread::spawn(move || {
            for _ in 0..requests {
                let (mut socket, _) = listener.accept().unwrap();

                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut line = String::new();

                BufReader::new(socket.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                let request: RuntimeRequest = serde_json::from_str(&line).unwrap();
                let result = Ok::<_, rake_protocol::control::ControlError>(reply(request));

                writeln!(socket, "{}", serde_json::to_string(&result).unwrap()).unwrap();
            }
        });

        (instance, worker, path)
    }

    #[test]
    fn source_links_share_a_file_identity_across_captured_sites() {
        let info = rake::router::Router::<rake_protocol::router::NoRoute>::new(Default::default())
            .client()
            .info();
        let (instance, worker, path) =
            fixture(|_| panic!("source catalog needs no runtime command"), 0);
        let mut session = Session::new(instance, None);
        session
            .sources
            .bind("http://localhost/token", &info.runtime_id.to_string());
        session.domains.insert("Page".into());
        let mut record: rake_diagnostics::Record = serde_json::from_value(json!({"sequence":1,"elapsed_us":0,"level":"INFO","target":"test","kind":"event","spans":[],"source":{"file":"src/cdp/session.rs","line":2},"fields":{}})).unwrap();
        let first = session.source_link(&record).unwrap();
        assert!(
            session.outbound.is_empty(),
            "wait for Chrome's root resource tree"
        );
        session.source_tree_loaded = true;
        session.publish_sources();
        record.sequence = 9;
        record.source.as_mut().unwrap().line = 20;
        assert_eq!(session.source_link(&record).unwrap(), first);
        assert_eq!(
            session.outbound.len(),
            2,
            "one source document, not one per diagnostic"
        );
        let descriptor = session.source_descriptor(&rake_protocol::inspection::SourceLocation {
            file: "src/cdp/session.rs".into(),
            line: 20,
            column: 3,
        });
        assert_eq!(descriptor["url"], first);
        assert_eq!(descriptor["lineNumber"], 19);
        assert_eq!(descriptor["columnNumber"], 2);
        session.event(Event::new(
            "oversized",
            json!({"text":"x".repeat(1024*1024)}),
        ));
        assert_eq!(
            session.sources.events().len(),
            2,
            "dropped output schedules source documents for resynchronization"
        );
        let mut successor = session.instance.clone();
        successor.info.runtime_id = rake_protocol::router::RuntimeId(uuid::Uuid::new_v4());
        session.replace(successor);
        assert!(
            session
                .outbound
                .iter()
                .any(|(event, _)| event.method == "Page.frameDetached"
                    && event.params["frameId"] == format!("rake-source:{first}"))
        );
        worker.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn automatic_report_event_completion_waits_for_chrome_stop() {
        let (instance, worker, path) = fixture(
            |request| match request.message {
                RuntimeMessage::Command(Command::RecordFrames { enabled: false, .. }) => {
                    Reply::Data(json!("runtime unavailable"))
                }
                _ => panic!("unexpected request"),
            },
            1,
        );
        let mut session = Session::new(instance, None);
        session
            .frame_trace
            .begin(&json!({"transferMode":"ReportEvents"}))
            .unwrap();
        session.finish_trace(Some("runtime replaced"));
        assert!(!session.frame_trace.active);
        assert!(session.pending_trace_complete.is_some());
        assert!(
            !session
                .outbound
                .iter()
                .any(|(event, _)| event.method == "Tracing.tracingComplete")
        );
        let request = |method: &str| Request {
            id: 1,
            method: method.into(),
            params: json!({}),
        };
        assert!(session.handle(&request("Tracing.start")).is_err());
        session.handle(&request("Tracing.end")).unwrap();
        assert!(session.pending_trace_complete.is_none());
        let complete = &session.outbound.back().unwrap().0;
        assert_eq!(complete.method, "Tracing.tracingComplete");
        assert_eq!(complete.params["dataLossOccurred"], true);
        worker.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn baseline_failure_releases_recording_and_dead_stop_returns_a_partial_trace() {
        for bad_baseline in [true, false] {
            let (instance, worker, path) = fixture(
                move |request| match request.message {
                    RuntimeMessage::Command(Command::RecordFrames { enabled, .. }) => {
                        if !bad_baseline && !enabled {
                            Reply::Data(json!("runtime unavailable"))
                        } else {
                            Reply::Acknowledged
                        }
                    }
                    RuntimeMessage::Query(Query::Frames { .. }) => {
                        if bad_baseline {
                            Reply::Data(json!("unexpected baseline"))
                        } else {
                            Reply::Frames(rake_protocol::inspection::Frames {
                                clock: None,
                                recording: true,
                                frames: vec![],
                                cursor: 0,
                                more: false,
                                gap: false,
                                dropped: 0,
                            })
                        }
                    }
                    _ => panic!("unexpected request"),
                },
                3,
            );
            let mut session = Session::new(instance, None);
            let start = session.handle(&Request {
                id: 1,
                method: "Tracing.start".into(),
                params: json!({"transferMode":"ReturnAsStream"}),
            });
            if bad_baseline {
                assert!(start.is_err());
                assert!(!session.frame_trace.active);
            } else {
                start.unwrap();
                session
                    .handle(&Request {
                        id: 2,
                        method: "Tracing.end".into(),
                        params: json!({}),
                    })
                    .unwrap();
                assert_eq!(
                    session.outbound.back().unwrap().0.params["dataLossOccurred"],
                    true
                );
                assert!(session.frame_trace.has_stream());
            }
            worker.join().unwrap();
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn chrome_frame_recording_streams_actual_runtime_samples_and_stops_cleanly() {
        let (instance, worker, path) = fixture(
            |request| match request.message {
                RuntimeMessage::Command(Command::RecordFrames { .. }) => Reply::Acknowledged,
                RuntimeMessage::Query(Query::Frames { after }) => {
                    Reply::Frames(rake_protocol::inspection::Frames {
                        clock: None,
                        recording: true,
                        frames: if after == 0 {
                            vec![rake_protocol::inspection::FrameTiming {
                                sequence: 1,
                                window: rake_protocol::router::WindowId(1),
                                elapsed_us: 100,
                                draw_us: 20,
                                dirty_to_draw_us: Some(25),
                                invalidations: 2,
                                nodes: Vec::new(),
                                omitted_nodes: 0,
                            }]
                        } else {
                            Vec::new()
                        },
                        cursor: if after == FRAME_HIGH_WATER_CURSOR {
                            0
                        } else {
                            1
                        },
                        more: false,
                        gap: false,
                        dropped: 0,
                    })
                }
                _ => panic!("unexpected tracing request"),
            },
            6,
        );
        let mut session = Session::new(instance, None);
        let request = |method: &str, params| Request {
            id: 1,
            method: method.into(),
            params,
        };
        session
            .handle(&request(
                "Tracing.start",
                json!({"transferMode":"ReturnAsStream"}),
            ))
            .unwrap();
        session.poll_frames(true).unwrap();
        session.handle(&request("Tracing.end", json!({}))).unwrap();
        assert_eq!(
            session.outbound.back().unwrap().0.method,
            "Tracing.tracingComplete"
        );
        let read = session
            .handle(&request(
                "IO.read",
                json!({"handle":session.outbound.back().unwrap().0.params["stream"]}),
            ))
            .unwrap();
        let trace: Value = serde_json::from_str(read["data"].as_str().unwrap()).unwrap();
        assert!(
            trace["traceEvents"]
                .as_array()
                .unwrap()
                .iter()
                .any(|event| event["name"] == "RunTask" && event["dur"] == 20)
        );
        assert!(!session.frame_trace.active);
        worker.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn style_edits_cross_runtime_once_and_never_edit_authored_sheets() {
        let mut snapshot = capture("style-edit");
        snapshot.elements[0].style_editable = true;
        let (instance, worker, path) = fixture(
            move |request| match request.message {
                RuntimeMessage::Query(Query::InspectTree {}) => Reply::Elements(snapshot.clone()),
                RuntimeMessage::Command(Command::EditElementStyle {
                    id,
                    generation,
                    edit,
                }) => {
                    assert_eq!(id, "element-style-edit-0");
                    assert_eq!(generation, "style-edit");
                    assert!(request.key.is_none());
                    match edit {
                        rake_protocol::inspection::StyleEdit::Set { declaration } => {
                            snapshot.elements[0].style_overrides = vec![declaration]
                        }
                        rake_protocol::inspection::StyleEdit::Reset => {
                            snapshot.elements[0].style_overrides.clear()
                        }
                        _ => panic!("unexpected edit"),
                    }
                    Reply::Acknowledged
                }
                _ => panic!("unexpected style request"),
            },
            6,
        );
        let mut session = Session::new(instance, None);
        session.dom.update(capture("style-edit")).unwrap();
        let node = session.dom.node_id("element-style-edit-0").unwrap();
        let request = |method: &str, params| Request {
            id: 1,
            method: method.into(),
            params,
        };
        session
            .handle(&request(
                "CSS.setEffectivePropertyValueForNode",
                json!({"nodeId":node,"propertyName":"color","value":"#ff0000"}),
            ))
            .unwrap();
        assert!(
            session
                .dom
                .edit_text(node)
                .unwrap()
                .contains("color: #ff0000")
        );
        session
            .handle(&request("Rake.resetStyles", json!({"nodeId":node})))
            .unwrap();
        assert_eq!(session.dom.edit_text(node).unwrap(), "");
        worker.join().unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(
            session
                .handle(&request(
                    "CSS.setStyleSheetText",
                    json!({"styleSheetId":"rake-resolved","text":"color: red"})
                ))
                .is_err()
        );
    }

    #[test]
    fn selected_element_console_handle_validates_context_and_source_targets() {
        let snapshot = capture("selected");
        let (instance, worker, path) = fixture(
            move |request| match request.message {
                RuntimeMessage::Query(Query::InspectTree {}) => Reply::Elements(snapshot.clone()),
                _ => panic!("unexpected selected-node request"),
            },
            1,
        );
        let mut session = Session::new(instance, None);
        session.dom.update(capture("selected")).unwrap();
        let node = session.dom.node_id("element-selected-0").unwrap();
        let call = |method: &str, params| Request {
            id: 1,
            method: method.into(),
            params,
        };
        session
            .handle(&call("DOM.setInspectedNode", json!({"nodeId":node})))
            .unwrap();
        assert!(
            session
                .handle(&call(
                    "Runtime.evaluate",
                    json!({"expression":"$0","contextId":99})
                ))
                .is_err()
        );
        let result = session.handle(&call("Runtime.evaluate", json!({"expression":"$0","contextId":1,"throwOnSideEffect":true,"objectGroup":"console"}))).unwrap();
        assert_eq!(
            session
                .dom
                .object_node(result["result"]["objectId"].as_str().unwrap())
                .unwrap(),
            node
        );
        assert_eq!(session.source_selector("$0").unwrap(), json!(node));
        assert_eq!(
            session
                .source_selector("$0 , {\"kind\":\"handler\",\"index\":0}")
                .unwrap(),
            json!({"nodeId":node,"target":{"kind":"handler","index":0}})
        );
        for invalid in ["$0.id", "$0, other()", "$0; evil()", "$01"] {
            assert!(session.source_selector(invalid).is_err());
        }
        worker.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn chrome_text_edits_and_selected_reset_undo_cross_runtime_once_without_operation_keys() {
        use rake_protocol::inspection::TextEdit;
        let mut snapshot = capture("edit-text");
        snapshot.elements[0].text = Some("Original".into());
        snapshot.elements[0].text_editable = true;
        let returned = snapshot.clone();
        let mut mutation = 0;
        let (instance, worker, path) = fixture(
            move |request| match request.message {
                RuntimeMessage::Query(Query::InspectTree {}) => Reply::Elements(returned.clone()),
                RuntimeMessage::Command(Command::EditElementText {
                    id,
                    generation,
                    edit,
                }) => {
                    assert!(request.key.is_none());
                    assert_eq!(id, "element-edit-text-0");
                    assert_eq!(generation, "edit-text");
                    let expected = [
                        TextEdit::Set {
                            text: "café 你好 🙂".into(),
                        },
                        TextEdit::Reset {},
                        TextEdit::Undo {},
                    ];
                    assert_eq!(edit, expected[mutation]);
                    mutation += 1;
                    Reply::Acknowledged
                }
                _ => panic!("unexpected text edit request"),
            },
            6,
        );
        let mut session = Session::new(instance, None);
        session.dom.update(snapshot).unwrap();
        let node = session.dom.node_id("element-edit-text-0").unwrap();
        let call = |method: &str, params| Request {
            id: 1,
            method: method.into(),
            params,
        };
        session
            .handle(&call("DOM.setInspectedNode", json!({"nodeId":node})))
            .unwrap();
        session
            .handle(&call(
                "DOM.setNodeValue",
                json!({"nodeId":node,"value":"café 你好 🙂"}),
            ))
            .unwrap();
        session
            .handle(&call(
                "Runtime.evaluate",
                json!({"expression":"rake.resetText($0)"}),
            ))
            .unwrap();
        session
            .handle(&call(
                "Runtime.evaluate",
                json!({"expression":"rake.undoText($0)"}),
            ))
            .unwrap();
        worker.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn source_targets_load_exact_details_omitted_from_the_paged_index() {
        let mut snapshot = capture("source");
        snapshot.elements[0].semantic_id = Some("root".into());
        snapshot.elements[0].view = Some(serde_json::from_value(json!({"tag":"div","style_sources":[{"scope":"div","attribute":"class","expression":"p-2","source":{"file":"src/view.rs","line":20,"column":4}}]})).unwrap());
        snapshot.elements[0].handlers = vec![serde_json::from_value(json!({"name":"click","origin":"view","source":{"file":"src/view.rs","line":21,"column":7}})).unwrap()];
        let (mut instance, worker, path) = fixture(
            move |request| match request.message {
                RuntimeMessage::Query(Query::InspectTreePage { .. }) => {
                    let mut page = snapshot.clone();
                    page.elements[0]
                        .view
                        .as_mut()
                        .unwrap()
                        .style_sources
                        .clear();
                    page.elements[0].handlers.clear();
                    Reply::ElementPage(rake_protocol::inspection::TreePage {
                        total: page.elements.len(),
                        snapshot: page,
                        next: None,
                    })
                }
                RuntimeMessage::Query(Query::InspectElement { id }) => Reply::Element(
                    snapshot
                        .elements
                        .iter()
                        .find(|e| e.id == id)
                        .unwrap()
                        .clone(),
                ),
                _ => panic!("unexpected source request"),
            },
            4,
        );
        instance.info.queries.push("inspect_tree_page".into());
        let mut session = Session::new(instance, None);
        let style = session
            .source_location(&json!({"nodeId":"root","target":{"kind":"style","index":0}}))
            .unwrap();
        let handler = session
            .source_location(&json!({"nodeId":"root","target":{"kind":"handler","index":0}}))
            .unwrap();
        assert_eq!((style.line, style.column), (20, 4));
        assert_eq!((handler.line, handler.column), (21, 7));
        worker.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn paged_index_loads_large_trees_and_fetches_details_only_on_demand() {
        let mut snapshot = capture("paged");
        let template = snapshot.elements[0].clone();
        for i in 1..600 {
            let mut element = template.clone();
            element.id = format!("element-paged-{i}");
            element.stable_id = Some(format!("boundary-{i}"));
            snapshot.elements.push(element);
        }
        let (mut instance, worker, path) = fixture(
            move |request| match request.message {
                RuntimeMessage::Query(Query::InspectTreePage { generation, offset }) => {
                    assert!(generation.is_none() || generation.as_deref() == Some("paged"));
                    let end = (offset + 256).min(600);
                    let mut page = snapshot.clone();
                    page.elements = snapshot.elements[offset..end].to_vec();
                    Reply::ElementPage(rake_protocol::inspection::TreePage {
                        snapshot: page,
                        total: 600,
                        next: (end < 600).then_some(end),
                    })
                }
                RuntimeMessage::Query(Query::InspectElement { id }) => Reply::Element(
                    snapshot
                        .elements
                        .iter()
                        .find(|e| e.id == id)
                        .unwrap()
                        .clone(),
                ),
                _ => panic!("unexpected paged request"),
            },
            4,
        );
        instance.info.queries.push("inspect_tree_page".into());
        let mut session = Session::new(instance, None);
        session.refresh_dom().unwrap();
        let node = session.dom.node_id("element-paged-599").unwrap();
        session.load_details(node).unwrap();
        assert_eq!(session.dom.search("Div").unwrap()["resultCount"], 600);
        worker.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn closing_a_session_clears_picker_and_highlight_without_retaining_retry_keys() {
        let observed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let commands = observed.clone();

        let (instance, worker, path) = fixture(
            move |request| {
                assert!(request.key.is_none());

                let RuntimeMessage::Command(command) = request.message else {
                    panic!("control command");
                };

                commands.lock().unwrap().push(command);

                Reply::Acknowledged
            },
            3,
        );

        let mut session = Session::new(instance, None);

        session
            .handle(&Request {
                id: 1,
                method: "Overlay.setInspectMode".into(),
                params: json!({"mode":"searchForNode"}),
            })
            .unwrap();
        drop(session);
        worker.join().unwrap();
        assert_eq!(
            *observed.lock().unwrap(),
            vec![
                Command::InspectMode {
                    enabled: true,
                    lease_ms: 1000
                },
                Command::InspectMode {
                    enabled: false,
                    lease_ms: 0
                },
                Command::HighlightElement {
                    id: None,
                    generation: String::new()
                }
            ]
        );
        std::fs::remove_file(path).unwrap();
    }

    fn capture(generation: &str) -> rake_protocol::inspection::Snapshot {
        serde_json::from_value(json!({"status":"captured","capture":generation,"router_revision":0,"elements":[{
            "id":format!("element-{generation}-0"),"kind":"Div","metadata_truncated":false}],"omitted":0,"coverage":"test"})).unwrap()
    }

    #[test]
    fn expand_then_reveal_keeps_the_frontend_tree_and_selection_intact() {
        let mut snapshot = capture("stable");
        let mut child = snapshot.elements[0].clone();

        child.id = "element-stable-1".into();
        child.parent = Some("element-stable-0".into());
        snapshot.elements.push(child);
        let (instance, worker, path) = fixture(move |_| Reply::Elements(snapshot.clone()), 1);
        let mut session = Session::new(instance, None);

        let request = |method: &str, params| Request {
            id: 1,
            method: method.into(),
            params,
        };

        let doc = session
            .handle(&request("DOM.getDocument", json!({})))
            .unwrap();

        let root = doc["root"]["children"][0]["nodeId"].as_i64().unwrap();

        assert!(session.domains.contains("DOM"));
        session
            .handle(&request("DOM.requestChildNodes", json!({"nodeId":root})))
            .unwrap();
        let event = session.outbound.pop_front().unwrap().0;

        session.outbound_bytes = 0;
        assert_eq!(event.method, "DOM.setChildNodes");
        assert_eq!(event.params["parentId"], root);
        let child = event.params["nodes"][0]["backendNodeId"].as_i64().unwrap();

        let result = session
            .handle(&request(
                "DOM.pushNodesByBackendIdsToFrontend",
                json!({"backendNodeIds":[child]}),
            ))
            .unwrap();

        assert_eq!(result["nodeIds"], json!([child]));
        assert!(
            session.outbound.is_empty(),
            "known node reveal must not replace any Chrome DOM objects"
        );
        session
            .handle(&request(
                "DOM.requestChildNodes",
                json!({"nodeId":root,"depth":-1}),
            ))
            .unwrap();
        assert!(session.outbound.is_empty());
        worker.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn live_style_events_invalidate_chrome_without_document_resets() {
        let mut snapshot = capture("live");

        snapshot.elements[0].resolved = Some(rake_protocol::inspection::ResolvedStyle {
            properties: json!({"opacity":1.0}),
            hovered: false,
            layers: Vec::new(),
        });
        let mut initial = true;

        let (instance, worker, path) = fixture(
            move |_| {
                if initial {
                    initial = false;
                } else {
                    snapshot.elements[0].resolved.as_mut().unwrap().properties["opacity"] =
                        json!(0.5);
                    snapshot.elements[0].resolved.as_mut().unwrap().hovered = true;
                }

                Reply::Elements(snapshot.clone())
            },
            2,
        );

        let mut session = Session::new(instance, None);

        let request = |method: &str, params| Request {
            id: 1,
            method: method.into(),
            params,
        };

        session.handle(&request("CSS.enable", json!({}))).unwrap();
        assert_eq!(
            session.outbound.front().unwrap().0.method,
            "CSS.styleSheetAdded"
        );
        let doc = session
            .handle(&request("DOM.getDocument", json!({})))
            .unwrap();

        let id = doc["root"]["children"][0]["nodeId"].as_i64().unwrap();

        session
            .handle(&request(
                "CSS.trackComputedStyleUpdatesForNode",
                json!({"nodeId":id}),
            ))
            .unwrap();
        session.outbound.clear();
        session.outbound_bytes = 0;
        session.refresh_dom().unwrap();
        let events: Vec<_> = session
            .outbound
            .iter()
            .map(|(event, _)| event.method.as_str())
            .collect();

        assert_eq!(
            events,
            ["CSS.styleSheetChanged", "CSS.computedStyleUpdated"]
        );
        assert_eq!(session.outbound.back().unwrap().0.params["nodeId"], id);
        let text = session
            .handle(&request(
                "CSS.getStyleSheetText",
                json!({"styleSheetId":"rake-resolved"}),
            ))
            .unwrap();

        assert!(text["text"].as_str().unwrap().contains("0.5"));
        worker.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn search_and_chrome_focus_helper_cross_the_runtime_protocol_without_evaluation() {
        let mut snapshot = capture("navigation");
        snapshot.elements[0].semantic_id = Some("save".into());
        snapshot.elements[0].navigation.focus = true;
        let (instance, worker, path) = fixture(
            move |request| match request.message {
                RuntimeMessage::Query(Query::InspectTree {}) => Reply::Elements(snapshot.clone()),
                RuntimeMessage::Command(Command::NavigateElement {
                    id,
                    generation,
                    action,
                }) => {
                    assert_eq!(id, "element-navigation-0");
                    assert_eq!(generation, "navigation");
                    assert_eq!(action, rake_protocol::inspection::NavigationAction::Focus);
                    assert!(request.key.is_none());
                    Reply::Acknowledged
                }
                _ => panic!("Unexpected navigation request"),
            },
            6,
        );
        let mut session = Session::new(instance, None);
        let call = |method: &str, params| Request {
            id: 1,
            method: method.into(),
            params,
        };
        session.handle(&call("DOM.getDocument", json!({}))).unwrap();
        let search = session
            .handle(&call("DOM.performSearch", json!({"query":"#save"})))
            .unwrap();
        let result = session
            .handle(&call(
                "DOM.getSearchResults",
                json!({"searchId":search["searchId"],"fromIndex":0,"toIndex":1}),
            ))
            .unwrap();
        let object = session
            .handle(&call(
                "DOM.resolveNode",
                json!({"nodeId":result["nodeIds"][0]}),
            ))
            .unwrap();
        let result = session.handle(&call("Runtime.callFunctionOn", json!({"objectId":object["object"]["objectId"],"functionDeclaration":"function focusInPage(){ this.focus(); }"}))).unwrap();
        assert_eq!(result["result"]["type"], "undefined");
        worker.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn incremental_capture_updates_preserve_tracking_and_renew_highlights_with_current_control_ids()
    {
        let mut initial = capture("1");
        initial.elements[0].stable_id = Some("root".into());
        initial.elements[0].layout = Some(rake_protocol::inspection::Layout {
            x: 0.,
            y: 0.,
            width: 10.,
            height: 10.,
        });
        let mut next = capture("2");
        next.elements[0].stable_id = Some("root".into());
        next.elements[0].layout = Some(rake_protocol::inspection::Layout {
            x: 0.,
            y: 0.,
            width: 42.,
            height: 10.,
        });
        let mut child = next.elements[0].clone();
        child.id = "element-2-1".into();
        child.stable_id = Some("child".into());
        child.parent = Some("element-2-0".into());
        next.elements.push(child);
        let (instance, worker, path) = fixture(
            move |request| match request.message {
                RuntimeMessage::Query(Query::InspectTree {}) => Reply::Elements(next.clone()),
                RuntimeMessage::Command(Command::HighlightElement { id, generation }) => {
                    assert_eq!(id.as_deref(), Some("element-2-0"));
                    assert_eq!(generation, "2");
                    Reply::Acknowledged
                }
                _ => panic!("unexpected runtime request"),
            },
            2,
        );
        let mut session = Session::new(instance, None);
        session.dom.update(initial).unwrap();
        session.dom.document(&json!({"depth":-1})).unwrap();
        let root = session.dom.node_id("element-1-0").unwrap();
        session.domains.extend(["DOM".into(), "CSS".into()]);
        session.tracked_style_node = Some(root);
        session.highlight = Some(root);

        session.refresh_dom().unwrap();
        let events = session
            .outbound
            .iter()
            .map(|(event, _)| event.method.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            events,
            [
                "DOM.childNodeInserted",
                "CSS.styleSheetChanged",
                "CSS.computedStyleUpdated"
            ]
        );
        assert_eq!(session.dom.node_id("element-2-0"), Some(root));
        assert_eq!(session.tracked_style_node, Some(root));
        session.poll_overlay().unwrap();
        assert_eq!(session.highlight, Some(root));
        worker.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn oversized_mutation_batches_resynchronize_instead_of_dropping_part_of_the_tree() {
        let mut initial = capture("1");
        initial.elements[0].stable_id = Some("root".into());
        let mut next = capture("2");
        next.elements[0].stable_id = Some("root".into());
        let mut child = next.elements[0].clone();
        child.id = "element-2-1".into();
        child.stable_id = Some("child".into());
        child.parent = Some("element-2-0".into());
        next.elements.push(child);
        let (instance, worker, path) = fixture(move |_| Reply::Elements(next.clone()), 1);
        let mut session = Session::new(instance, None);
        session.dom.update(initial).unwrap();
        session.dom.document(&json!({"depth":-1})).unwrap();
        session.domains.insert("DOM".into());
        for _ in 0..QUEUE_LIMIT {
            session.event(Event::new("Log.entryAdded", json!({})));
        }

        session.refresh_dom().unwrap();
        assert_eq!(session.outbound.len(), 1);
        assert_eq!(session.outbound[0].0.method, "DOM.documentUpdated");
        assert_eq!(session.dropped, QUEUE_LIMIT as u64);
        let document = session.dom.document(&json!({"depth":-1})).unwrap();
        assert_eq!(
            document["children"][0]["children"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        worker.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn picker_generation_mismatch_updates_document_before_pick_without_unknown_hover_ids() {
        let (instance, worker, path) = fixture(
            |request| match request.message {
                RuntimeMessage::Query(Query::InspectPick {}) => {
                    Reply::InspectPick(rake_protocol::inspection::Pick {
                        mode: false,
                        generation: "2".into(),
                        hovered: Some("element-2-0".into()),
                        picked: Some("element-2-0".into()),
                        pick_sequence: 1,
                    })
                }
                RuntimeMessage::Query(Query::InspectTree {}) => Reply::Elements(capture("2")),
                _ => panic!("unexpected picker request"),
            },
            2,
        );

        let mut session = Session::new(instance, None);

        session.dom.update(capture("1")).unwrap();
        session.domains.insert("DOM".into());
        session.picking = true;
        session.poll_overlay().unwrap();
        let events: Vec<_> = session
            .outbound
            .iter()
            .map(|(event, _)| event.method.as_str())
            .collect();

        assert_eq!(
            events,
            ["DOM.documentUpdated", "Overlay.inspectNodeRequested"]
        );
        let id = session.dom.node_id("element-2-0").unwrap();

        assert_eq!(
            session.outbound.back().unwrap().0.params["backendNodeId"],
            id
        );
        assert!(!session.picking);
        worker.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }
}
