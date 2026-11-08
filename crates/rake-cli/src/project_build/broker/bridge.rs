//! Session-owned browser leases and redeliverable request tickets.
use super::{Error, MAX_BYTES, Result};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Condvar, Mutex},
    time::{Duration, Instant},
};

pub(super) struct Instance {
    pub(super) seen: Instant,
    build_id: u64,
    reload: Value,
    error: Value,
    attempted: u64,
    pub(super) retry_after: Instant,
}

pub(super) struct Ticket {
    pub(super) id: String,
    pub(super) runtime: String,
    pub(super) request: Value,
    pub(super) result: Option<Value>,
    pub(super) created: Instant,
}

#[derive(Default)]
pub(super) struct State {
    pub(super) instances: BTreeMap<String, Instance>,
    pub(super) tickets: Vec<Ticket>,
    manifests: BTreeSet<PathBuf>,
    closed: bool,
}

pub(super) struct Bridge {
    pub(super) token: String,
    pub(super) directory: PathBuf,
    pub(super) state: Mutex<State>,
    condition: Condvar,
    url: String,
    pub(super) development: Option<PathBuf>,
}

impl Bridge {
    pub(super) fn new(
        url: String,
        directory: PathBuf,
        development: Option<PathBuf>,
    ) -> Result<Self> {
        rake_client::local::prepare_directory(&directory)?;
        Ok(Self {
            token: super::token(),
            directory,
            state: Mutex::default(),
            condition: Condvar::new(),
            url,
            development,
        })
    }

    pub(super) fn development(&self) -> Value {
        let Some(path) = &self.development else {
            return json!({"published":0});
        };
        fs::File::open(path)
            .ok()
            .and_then(|file| serde_json::from_reader(file.take(MAX_BYTES as u64)).ok())
            .unwrap_or_else(|| json!({"published":0,"phase":"starting"}))
    }

    fn expire(&self, state: &mut State) -> Result<()> {
        state
            .tickets
            .retain(|ticket| ticket.created.elapsed() <= Duration::from_secs(60));
        let expired: Vec<_> = state
            .instances
            .iter()
            .filter(|(_, instance)| instance.seen.elapsed() > Duration::from_secs(30))
            .map(|(runtime, _)| runtime.clone())
            .collect();
        for runtime in expired {
            state.instances.remove(&runtime);
            let path = self.directory.join(format!("{runtime}.json"));
            if state.manifests.remove(&path) {
                remove(&path)?;
            }
        }
        Ok(())
    }

    pub(super) fn close(&self) {
        let mut state = self.state.lock().expect("bridge state lock poisoned");
        state.closed = true;
        for path in std::mem::take(&mut state.manifests) {
            let _ = remove(&path);
        }
        state.instances.clear();
        self.condition.notify_all();
    }

    pub(super) fn reload(&self, runtime: &str) -> Value {
        self.state
            .lock()
            .expect("bridge state lock poisoned")
            .instances
            .get(runtime)
            .map(|instance| instance.reload.clone())
            .unwrap_or(Value::Null)
    }

    pub(super) fn contains(&self, runtime: &str) -> bool {
        self.state
            .lock()
            .expect("bridge state lock poisoned")
            .instances
            .contains_key(runtime)
    }

    pub(super) fn handle(&self, action: &str, body: &Value) -> Result<Value> {
        let mut state = self.state.lock().expect("bridge state lock poisoned");
        self.expire(&mut state)?;
        if state.closed {
            return Err(Error::new(503, "Development bridge stopped"));
        }
        if action == "development" {
            let mut development = self.development();
            if !development.is_object() {
                return Err(Error::new(400, "invalid development state"));
            }
            development["runtimes"] = state.instances.iter().map(|(id, instance)| (id.clone(),json!({"build_id":instance.build_id,"reload":instance.reload,"error":instance.error}))).collect::<serde_json::Map<_,_>>().into();
            return Ok(development);
        }
        if action == "connect" {
            let info = body.get("info").cloned().unwrap_or_else(|| json!({}));
            if !info.is_object() {
                return Err(Error::new(400, "invalid runtime information"));
            }
            let runtime = info["runtime_id"]
                .as_str()
                .and_then(|id| uuid::Uuid::parse_str(id).ok())
                .ok_or_else(|| Error::new(400, "invalid runtime ID"))?
                .to_string();
            if info["protocol"] != 1 || info["platform"] != "web" {
                return Err(Error::new(400, "unsupported browser protocol/platform"));
            }
            if !state.instances.contains_key(&runtime) && state.instances.len() >= 32 {
                return Err(Error::new(429, "browser instance capacity reached"));
            }
            let app = body["application_id"]
                .as_str()
                .filter(|id| !id.is_empty() && id.chars().count() <= 256)
                .ok_or_else(|| Error::new(400, "invalid application ID"))?;
            let build_id = body
                .get("build_id")
                .unwrap_or(&Value::from(0))
                .as_u64()
                .filter(|build| *build <= 1 << 53)
                .ok_or_else(|| Error::new(400, "invalid build ID"))?;
            let now = Instant::now();
            let instance = state
                .instances
                .entry(runtime.clone())
                .or_insert_with(|| Instance {
                    seen: now,
                    build_id: 0,
                    reload: Value::Null,
                    error: Value::Null,
                    attempted: 0,
                    retry_after: now,
                });
            instance.seen = now;
            instance.build_id = build_id;
            instance.reload = body["reload"].clone();
            let manifest = json!({"application_id":app,"pid":std::process::id(),"info":info,"endpoint":{"type":"web","url":self.url,"token":self.token}});
            let path = self.directory.join(format!("{runtime}.json"));
            super::super::write_marker(&path, &manifest)
                .map_err(|error| Error::new(502, error.to_string()))?;
            state.manifests.insert(path);
            return Ok(json!({"connected":true}));
        }
        let runtime = body["runtime_id"].as_str().unwrap_or_default();
        if !state.instances.contains_key(runtime) {
            return Err(Error::new(
                404,
                "browser instance unavailable; reconnect required",
            ));
        }
        match action {
            "reload_result" => {
                let instance = state
                    .instances
                    .get_mut(runtime)
                    .expect("validated instance");
                instance.reload = body["reload"].clone();
                instance.error = body["error"].clone();
                if body["error"]
                    .as_str()
                    .is_some_and(|error| error.contains("runtime is busy"))
                {
                    instance.attempted = 0;
                    instance.retry_after = Instant::now() + Duration::from_secs(1);
                }
                Ok(json!({"received":true}))
            }
            "poll" => {
                state
                    .instances
                    .get_mut(runtime)
                    .expect("validated instance")
                    .seen = Instant::now();
                let deadline = Instant::now() + Duration::from_secs(2);
                loop {
                    if state.closed {
                        return Err(Error::new(503, "Development bridge stopped"));
                    }
                    if let Some(ticket) = state
                        .tickets
                        .iter()
                        .find(|ticket| ticket.runtime == runtime && ticket.result.is_none())
                    {
                        return Ok(json!([{"ticket":ticket.id,"request":ticket.request}]));
                    }
                    let published = self.development()["published"].as_u64().unwrap_or(0);
                    let instance = state.instances.get_mut(runtime).ok_or_else(|| {
                        Error::new(404, "browser instance unavailable; reconnect required")
                    })?;
                    if published > instance.build_id
                        && published > instance.attempted
                        && Instant::now() >= instance.retry_after
                    {
                        instance.attempted = published;
                        return Ok(
                            json!([{"ticket":format!("reload-{published}"),"reload_build":published}]),
                        );
                    }
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Ok(json!([]));
                    }
                    state = self
                        .condition
                        .wait_timeout(state, remaining.min(Duration::from_millis(250)))
                        .expect("bridge state lock poisoned")
                        .0;
                }
            }
            "reply" => {
                if let Some(ticket) = state.tickets.iter_mut().find(|ticket| {
                    body["ticket"].as_str() == Some(&ticket.id) && ticket.runtime == runtime
                }) {
                    let result = &body["result"];
                    if !result.as_object().is_some_and(|object| {
                        object.len() == 1
                            && (object.contains_key("Ok") || object.contains_key("Err"))
                    }) {
                        return Err(Error::new(400, "invalid runtime reply"));
                    }
                    ticket.result = Some(result.clone());
                    self.condition.notify_all();
                }
                Ok(json!({"received":true}))
            }
            "client" => {
                let request = &body["request"];
                if !request.is_object()
                    || request["runtime_id"] != runtime
                    || request["protocol"] != 1
                {
                    return Err(Error::new(400, "request/runtime mismatch"));
                }
                if state.tickets.len() >= 128
                    || state
                        .tickets
                        .iter()
                        .filter(|ticket| ticket.runtime == runtime)
                        .count()
                        >= 32
                {
                    return Err(Error::new(429, "development broker queue full"));
                }
                let id = uuid::Uuid::new_v4().simple().to_string();
                state.tickets.push(Ticket {
                    id: id.clone(),
                    runtime: runtime.into(),
                    request: request.clone(),
                    result: None,
                    created: Instant::now(),
                });
                self.condition.notify_all();
                let deadline = Instant::now() + Duration::from_secs(10);
                loop {
                    if state.closed {
                        return Err(Error::new(503, "Development bridge stopped"));
                    }
                    if let Some(index) = state
                        .tickets
                        .iter()
                        .position(|ticket| ticket.id == id && ticket.result.is_some())
                    {
                        return Ok(state
                            .tickets
                            .remove(index)
                            .result
                            .expect("completed ticket"));
                    }
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        // A timeout does not cancel delivery; a retry retains the caller's key.
                        return Err(Error::new(
                            504,
                            "browser response timed out; retry with the same request key",
                        ));
                    }
                    state = self
                        .condition
                        .wait_timeout(state, remaining)
                        .expect("bridge state lock poisoned")
                        .0;
                }
            }
            _ => Err(Error::new(404, "unknown development bridge endpoint")),
        }
    }
}

fn remove(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}
