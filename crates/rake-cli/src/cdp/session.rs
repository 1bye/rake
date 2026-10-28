//! One connection owns its cursors, enabled domains and bounded event queue.

use super::wire::{Error, Event, Request, Response};
use super::{Result, TIMEOUT};
use crate::terminal::query;
use rake::control::local::Instance;
use rake::control::{Query, Reply};
use serde_json::{Value, json};
use std::collections::{BTreeSet, VecDeque};
use std::net::TcpStream;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tungstenite::{Message, WebSocket};

const INTERVAL: Duration = Duration::from_millis(250);
const QUEUE_LIMIT: usize = 256;

struct Session {
    instance: Instance,
    cursor: u64,
    domains: BTreeSet<String>,
    outbound: VecDeque<Event>,
    dropped: u64,
    epoch_seconds: Option<f64>,
    objects: super::console::Objects,
    loss: super::console::Loss,
    process_output: super::console::ProcessOutput,
    dom: super::dom::Dom,
}

impl Session {
    fn new(instance: Instance, output: Option<std::path::PathBuf>) -> Self {
        Self {
            instance,
            cursor: 0,
            domains: BTreeSet::new(),
            outbound: VecDeque::new(),
            dropped: 0,
            epoch_seconds: None,
            objects: super::console::Objects::default(),
            loss: super::console::Loss::default(),
            process_output: super::console::ProcessOutput::new(output),
            dom: super::dom::Dom::default(),
        }
    }

    fn event(&mut self, event: Event) {
        if self.outbound.len() == QUEUE_LIMIT {
            self.dropped += 1;
        } else {
            self.outbound.push_back(event);
        }
    }

    fn handle(&mut self, request: &Request) -> std::result::Result<Value, Error> {
        if !request.params.is_null() && !request.params.is_object() {
            return Err(Error::new(-32602, "params must be an object"));
        }

        let method = request.method.as_str();

        match method {
            "DOM.enable" | "CSS.enable" => {
                self.domains
                    .insert(method.split('.').next().unwrap_or_default().into());
                if method == "DOM.enable" {
                    self.warning("Rake layout uses the same bounds for all box quads; separate margin/padding metrics are unavailable. Styles are authored GPUI metadata.".into());
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
                Ok(json!({"root":self.dom.document()}))
            }
            "DOM.requestChildNodes" => {
                let node = self.dom.handle("DOM.describeNode", &request.params)?;
                self.event(Event::new(
                    "DOM.setChildNodes",
                    json!({"parentId":request.params["nodeId"],"nodes":node["node"]["children"]}),
                ));
                Ok(json!({}))
            }
            "DOM.getBoxModel"
            | "DOM.describeNode"
            | "DOM.getAttributes"
            | "DOM.pushNodesByBackendIdsToFrontend"
            | "CSS.getInlineStylesForNode"
            | "CSS.getMatchedStylesForNode"
            | "CSS.getComputedStyleForNode" => self.dom.handle(method, &request.params),
            "Overlay.highlightNode" | "Overlay.setInspectMode" => Err(Error::new(
                -32000,
                "Runtime highlighting is unavailable; a separate runtime protocol extension is required",
            )),
            "Overlay.enable" | "Overlay.disable" | "Overlay.hideHighlight" => Ok(json!({})),
            "Runtime.getProperties" => {
                let id = request.params["objectId"]
                    .as_str()
                    .ok_or_else(|| Error::new(-32602, "objectId required"))?;
                if request.params["accessorPropertiesOnly"] == true {
                    return Ok(json!({"result":[]}));
                }

                self.objects.properties(id)
            }
            "Runtime.releaseObject" => {
                if let Some(id) = request.params["objectId"].as_str() {
                    self.objects.release(id);
                }

                Ok(json!({}))
            }
            "Runtime.releaseObjectGroup" => {
                self.objects.clear();
                Ok(json!({}))
            }
            "Network.getResponseBody" | "Network.getRequestPostData" => Err(Error::new(
                -32000,
                "Request and response bodies are not captured by Rake",
            )),
            "Inspector.enable"
            | "Page.enable"
            | "Page.disable"
            | "Runtime.runIfWaitingForDebugger" => Ok(json!({})),
            "Network.enable" | "Log.enable" | "Runtime.enable" => {
                let domain = method.split('.').next().unwrap_or_default();
                let fresh = self.domains.insert(domain.into());

                if domain == "Runtime" && fresh {
                    self.event(Event::new("Runtime.executionContextCreated", json!({"context":{
                        "id":1, "origin":"rake://runtime", "name":self.instance.application_id,
                        "uniqueId":self.instance.info.runtime_id.to_string(), "auxData":{"isDefault":true,"frameId":"rake"}}})));
                }

                Ok(json!({}))
            }
            "Network.disable" | "Log.disable" | "Runtime.disable" => {
                self.domains
                    .remove(method.split('.').next().unwrap_or_default());

                Ok(json!({}))
            }
            "Page.getResourceTree" => Ok(json!({"frameTree":{"frame":frame(),"resources":[]}})),
            "Page.getFrameTree" => Ok(json!({"frameTree":{"frame":frame()}})),
            _ => Err(Error::new(
                -32601,
                format!("Unsupported CDP method: {method}"),
            )),
        }
    }

    fn poll(&mut self) -> Result<()> {
        match query(&self.instance, Query::Info {}, TIMEOUT)? {
            Reply::Info(info) if info.runtime_id == self.instance.info.runtime_id => {}
            _ => return Err("runtime identity changed".into()),
        }

        if self.domains.contains("DOM") {
            self.refresh_dom().map_err(|e| e.message)?;
        }

        let Reply::Diagnostics(snapshot) = query(
            &self.instance,
            Query::Diagnostics {
                after: self.cursor,
                kind: None,
            },
            TIMEOUT,
        )?
        else {
            return Err("unexpected diagnostics reply".into());
        };

        let warning = self.loss.observe(&snapshot, self.cursor);

        if let Some(record) = snapshot.records.last() {
            self.epoch_seconds
                .get_or_insert_with(|| unix_seconds() - record.elapsed_us as f64 / 1e6);
            self.cursor = record.sequence;
        }

        if self.domains.contains("Log") || self.domains.contains("Runtime") {
            if let Some(warning) = warning {
                self.warning(warning);
            }

            for record in &snapshot.records {
                if self.domains.contains("Log")
                    && let Some(event) =
                        super::console::translate(record, self.epoch_seconds.unwrap_or_default())
                {
                    self.event(event);
                }
                if self.domains.contains("Runtime") && record.kind == "event" {
                    let event = self
                        .objects
                        .called(record, self.epoch_seconds.unwrap_or_default());
                    self.event(event);
                }
            }

            if self.domains.contains("Log") {
                for event in self.process_output.poll(unix_seconds()) {
                    self.event(event);
                }
            }
        }

        if self.domains.contains("Network") {
            for record in &snapshot.records {
                for mut event in super::network::translate(record) {
                    if event.method == "Network.requestWillBeSent" {
                        event.params["wallTime"] = json!(
                            self.epoch_seconds.unwrap_or_default() + record.elapsed_us as f64 / 1e6
                        );
                    }

                    self.event(event);
                }
            }
        }

        Ok(())
    }

    fn refresh_dom(&mut self) -> std::result::Result<(), Error> {
        let reply = query(&self.instance, Query::InspectTree {}, TIMEOUT)
            .map_err(|e| Error::new(-32000, e.to_string()))?;
        let Reply::Elements(snapshot) = reply else {
            return Err(Error::new(-32000, "Unexpected inspection reply"));
        };
        if self.dom.update(snapshot)? && self.domains.contains("DOM") {
            self.event(Event::new("DOM.documentUpdated", json!({})));
        }
        Ok(())
    }

    fn replace(&mut self, instance: Instance) {
        let old = self.instance.info.runtime_id;
        self.instance = instance;
        self.cursor = 0;
        self.epoch_seconds = None;
        self.objects.clear();
        self.loss = super::console::Loss::default();
        self.dom.clear();
        self.outbound.clear();
        self.dropped = 0;
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
        let instances = rake::control::local::discover(directory).ok()?;
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

fn unix_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn frame() -> Value {
    json!({"id":"rake", "loaderId":"rake", "url":"rake://runtime", "domainAndRegistry":"",
        "securityOrigin":"rake://runtime", "mimeType":"text/plain", "secureContextType":"Secure",
        "crossOriginIsolatedContextType":"NotIsolated", "gatedAPIFeatures":[]})
}

pub(super) fn run(
    mut socket: WebSocket<TcpStream>,
    instance: Instance,
    output: Option<std::path::PathBuf>,
    directory: std::path::PathBuf,
) -> Result<()> {
    socket
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(20)))?;
    socket
        .get_mut()
        .set_write_timeout(Some(Duration::from_secs(1)))?;
    let mut session = Session::new(instance, output);
    let mut next_poll = Instant::now();
    let mut missing_since: Option<Instant> = None;

    loop {
        match socket.read() {
            Ok(Message::Text(text)) => {
                let response = match serde_json::from_str::<Request>(&text) {
                    Ok(request) => Response::new(Some(request.id), session.handle(&request)),
                    Err(error) => Response::new(None, Err(Error::new(-32600, error.to_string()))),
                };
                socket.send(Message::text(serde_json::to_string(&response)?))?;
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

        if !session.domains.is_empty() && Instant::now() >= next_poll {
            match session.poll() {
                Ok(()) => missing_since = None,
                Err(error) => {
                    if let Some(successor) = session.successor(&directory) {
                        session.replace(successor);
                        missing_since = None;
                    } else if missing_since.get_or_insert_with(Instant::now).elapsed()
                        >= Duration::from_secs(5)
                    {
                        socket.send(Message::text(serde_json::to_string(&Event::new(
                            "Inspector.detached",
                            json!({"reason":error.to_string()}),
                        ))?))?;
                        let _ = socket.close(None);
                        return Ok(());
                    }
                }
            }

            next_poll = Instant::now() + INTERVAL;
        }

        while let Some(event) = session.outbound.pop_front() {
            socket.send(Message::text(serde_json::to_string(&event)?))?;
        }

        if session.dropped > 0
            && (session.domains.contains("Log") || session.domains.contains("Runtime"))
        {
            let dropped = std::mem::take(&mut session.dropped);
            session.warning(format!("Rake CDP outbound queue dropped {dropped} events"));
        }
    }
}
