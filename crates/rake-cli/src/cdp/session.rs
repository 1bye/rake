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
}

impl Session {
    fn new(instance: Instance) -> Self {
        Self {
            instance,
            cursor: 0,
            domains: BTreeSet::new(),
            outbound: VecDeque::new(),
            dropped: 0,
            epoch_seconds: None,
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

        if let Some(record) = snapshot.records.last() {
            self.epoch_seconds
                .get_or_insert_with(|| unix_seconds() - record.elapsed_us as f64 / 1e6);
            self.cursor = record.sequence;
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

pub(super) fn run(mut socket: WebSocket<TcpStream>, instance: Instance) -> Result<()> {
    socket
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(20)))?;
    socket
        .get_mut()
        .set_write_timeout(Some(Duration::from_secs(1)))?;
    let mut session = Session::new(instance);
    let mut next_poll = Instant::now();

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
            if let Err(error) = session.poll() {
                socket.send(Message::text(serde_json::to_string(&Event::new(
                    "Inspector.detached",
                    json!({"reason": error.to_string()}),
                ))?))?;
                let _ = socket.close(None);

                return Ok(());
            }

            next_poll = Instant::now() + INTERVAL;
        }

        while let Some(event) = session.outbound.pop_front() {
            socket.send(Message::text(serde_json::to_string(&event)?))?;
        }
    }
}
