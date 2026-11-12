//! Native preview orchestration over the broker, with owned encoders and expiring references.
#[cfg(unix)]
mod capture;
#[cfg(windows)]
#[path = "capture_unavailable.rs"]
mod capture;
mod selection;
mod socket;

use super::broker::{Broker, Error};
use capture::Capture;
use serde_json::{Value, json};

use std::{
    collections::{HashMap, VecDeque},
    io::Write,
    net::TcpStream,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Error>;

pub(super) trait Inspector: Send + Sync {
    fn handle(&self, action: &str, body: &Value) -> Result<Value>;
}

impl Inspector for Broker {
    fn handle(&self, action: &str, body: &Value) -> Result<Value> {
        Broker::handle(self, action, body)
    }
}

#[derive(Clone)]
struct Reference {
    token: String,
    runtime: String,
    window: Value,
    created: Instant,
}

#[derive(Default)]
struct State {
    key: Option<Value>,
    epoch: Option<String>,
    captures: HashMap<String, Arc<Capture>>,
    references: VecDeque<Reference>,
    closed: bool,
}

impl State {
    fn stop(&mut self) {
        for (_, capture) in self.captures.drain() {
            capture.close();
        }

        self.key = None;
        self.epoch = None;
        self.references.clear();
    }
}

pub(super) struct Preview {
    inspector: Arc<dyn Inspector>,
    helper: PathBuf,
    state: Mutex<State>,
}

impl Preview {
    pub(super) fn new(inspector: Arc<dyn Inspector>, helper: PathBuf) -> Self {
        Self {
            inspector,
            helper,
            state: Mutex::default(),
        }
    }

    pub(super) fn close(&self) {
        let mut state = self.state.lock().expect("preview lock poisoned");

        state.closed = true;
        state.stop();
    }

    fn request(&self, runtime: &str, value: Value, command: bool) -> Result<Value> {
        let reply=self.inspector.handle("inspect",&json!({"request":{"protocol":1,"runtime_id":runtime,"key":null,"message":{"type":if command{"command"}else{"query"},"value":value}}}))?;

        if let Some(error) = reply.get("Err") {
            return Err(Error::new(
                409,
                format!(
                    "Preview changed or input unavailable: {}",
                    error.to_string().chars().take(512).collect::<String>()
                ),
            ));
        }

        Ok(reply["Ok"]["value"].clone())
    }

    fn reference(&self, body: &Value) -> Result<Reference> {
        self.state
            .lock()
            .expect("preview lock poisoned")
            .references
            .iter()
            .find(|reference| {
                body["reference"] == reference.token
                    && reference.created.elapsed() <= Duration::from_secs(5)
            })
            .cloned()
            .ok_or_else(|| Error::new(409, "Preview reference expired"))
    }

    fn state(&self) -> Result<Value> {
        let development = self.inspector.handle("development", &json!({}))?;

        let pid = if development["app_running"] == true {
            development["runtime_pid"].clone()
        } else {
            Value::Null
        };

        let instances = self.inspector.handle("inspect_instances", &json!({}))?;

        let instance = instances.as_array().and_then(|instances| {
            instances
                .iter()
                .find(|instance| instance["pid"] == pid && instance["transport"] == "local")
        });

        let Some(instance) = instance else {
            self.state.lock().expect("preview lock poisoned").stop();

            return Ok(
                json!({"error":"Waiting for the native application and its inspection host…"}),
            );
        };

        let runtime = instance["info"]["runtime_id"]
            .as_str()
            .ok_or_else(|| Error::new(502, "Invalid preview runtime"))?;

        let windows = self.request(runtime, json!({"type":"preview"}), false)?;

        let Some(window) = windows.as_array().and_then(|windows| windows.first()) else {
            self.state.lock().expect("preview lock poisoned").stop();

            return Ok(
                json!({"error":"Restore the inspected native window to start preview capture."}),
            );
        };

        // shortcut: one inspected window; add a chooser with multiwindow support.
        let key = json!([
            runtime,
            pid,
            window["native_id"],
            window["frame"],
            window["content"]
        ]);

        let mut state = self.state.lock().expect("preview lock poisoned");

        if state.closed {
            return Err(Error::new(503, "Preview stopped"));
        }

        if state.key.as_ref() != Some(&key) {
            state.stop();
            state.key = Some(key);
            state.epoch = Some(token());
        }

        let reference = token();

        state.references.push_back(Reference {
            token: reference.clone(),
            runtime: runtime.into(),
            window: window.clone(),
            created: Instant::now(),
        });

        while state.references.len() > 64 {
            state.references.pop_front();
        }

        let errors: serde_json::Map<_, _> = state
            .captures
            .iter()
            .filter_map(|(codec, capture)| {
                capture
                    .error()
                    .map(|error| (codec.clone(), Value::from(error)))
            })
            .collect();

        Ok(
            json!({"runtime_id":runtime,"window":window,"epoch":state.epoch,"reference":reference,"capture_errors":errors}),
        )
    }

    pub(super) fn handle(&self, action: &str, body: &Value) -> Result<Value> {
        if !body.is_object() {
            return Err(Error::new(400, "Expected preview object"));
        }

        match action {
            "preview_state" => self.state(),
            "preview_restart" => {
                self.state.lock().expect("preview lock poisoned").stop();

                Ok(Value::Null)
            }
            "preview_resolve" => self.resolve(body),
            "preview_input" | "preview_select" | "preview_hover" | "preview_grab" => {
                self.point(action, body)
            }
            _ => Err(Error::new(404, "Unknown preview endpoint")),
        }
    }

    fn point(&self, action: &str, body: &Value) -> Result<Value> {
        let coordinate = |key| {
            body[key]
                .as_f64()
                .filter(|value| value.is_finite())
                .ok_or_else(|| Error::new(400, "Finite content coordinates required"))
        };

        let (x, y) = (coordinate("x")?, coordinate("y")?);
        let saved = self.reference(body)?;
        let window = &saved.window;

        if !(x >= 0.
            && y >= 0.
            && x < window["content"]["width"].as_f64().unwrap_or(0.)
            && y < window["content"]["height"].as_f64().unwrap_or(0.))
        {
            return Err(Error::new(400, "Position is outside GPUI content"));
        }

        let mut point = fields(window, &["window", "native_id", "generation"]);

        point["x"] = json!(x.floor() as i64);
        point["y"] = json!(y.floor() as i64);

        if action != "preview_input" {
            return self.select(action, &saved.runtime, window, point);
        }

        let input = &body["input"];
        let kind = input["type"].as_str().unwrap_or_default();

        if !input.is_object() || !matches!(kind, "move" | "click" | "scroll" | "wheel") {
            return Err(Error::new(400, "Unsupported preview input"));
        }

        if matches!(kind, "scroll" | "wheel") {
            let limit = if kind == "wheel" { 128000 } else { 2000 };

            for axis in ["dx", "dy"] {
                if !input[axis]
                    .as_i64()
                    .is_some_and(|value| value >= -limit && value <= limit)
                {
                    return Err(Error::new(400, "Invalid or oversized scroll delta"));
                }
            }

            if kind == "wheel"
                && !matches!(input["phase"].as_str(), Some("started" | "moved" | "ended"))
            {
                return Err(Error::new(400, "Invalid scroll phase"));
            }
        }

        point["geometry"] = [
            ("frame", "width"),
            ("frame", "height"),
            ("content", "x"),
            ("content", "y"),
            ("content", "width"),
            ("content", "height"),
        ]
        .into_iter()
        .map(|(rect, key)| {
            json!((window[rect][key].as_f64().unwrap_or(0.) * 64.).round_ties_even() as i64)
        })
        .collect();

        self.request(
            &saved.runtime,
            json!({"type":"preview_input","point":point,"input":input}),
            true,
        )
    }

    pub(super) fn video(&self, stream: &mut TcpStream, body: &Value) -> Result<()> {
        let codec = body["codec"]
            .as_str()
            .filter(|codec| matches!(*codec, "h264" | "jpeg"))
            .ok_or_else(|| Error::new(400, "Expected h264 or jpeg codec"))?;

        let value = self.state()?;

        if let Some(error) = value["error"].as_str() {
            return Err(Error::new(409, error));
        }

        let (capture, viewer) = {
            let mut state = self.state.lock().expect("preview lock poisoned");

            if state.closed {
                return Err(Error::new(503, "Preview stopped"));
            }

            if state
                .captures
                .get(codec)
                .is_none_or(|capture| capture.error().is_some())
            {
                if let Some(old) = state.captures.remove(codec) {
                    old.close();
                }

                let pid = state
                    .key
                    .as_ref()
                    .and_then(|key| key[1].as_u64())
                    .ok_or_else(|| Error::new(409, "Preview changed"))?;

                let capture = Capture::start(
                    &self.helper,
                    pid,
                    &value["window"],
                    codec,
                    state
                        .epoch
                        .as_deref()
                        .ok_or_else(|| Error::new(409, "Preview changed"))?,
                )?;

                state.captures.insert(codec.into(), Arc::new(capture));
            }

            let capture = state.captures[codec].clone();
            let viewer = capture.viewer();

            (capture, viewer)
        };

        let result = (|| -> std::io::Result<()> {
            stream.set_write_timeout(Some(Duration::from_millis(500)))?;
            #[cfg(unix)]
            {
                use std::os::fd::AsRawFd;
                let size: libc::c_int = 64 * 1024;

                // SAFETY: stream is live; size points to the correctly sized socket option.
                if unsafe {
                    libc::setsockopt(
                        stream.as_raw_fd(),
                        libc::SOL_SOCKET,
                        libc::SO_SNDBUF,
                        (&size as *const libc::c_int).cast(),
                        std::mem::size_of_val(&size) as libc::socklen_t,
                    )
                } < 0
                {
                    return Err(std::io::Error::last_os_error());
                }
            }

            write!(
                stream,
                "HTTP/1.0 200 OK\r\nServer: rake\r\nContent-Type: application/octet-stream\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n"
            )?;

            viewer.stream(stream)
        })();

        let mut state = self.state.lock().expect("preview lock poisoned");

        drop(viewer);

        if capture.viewers() == 0
            && state
                .captures
                .get(codec)
                .is_some_and(|current| Arc::ptr_eq(current, &capture))
        {
            state.captures.remove(codec);
            capture.close();
        }

        // A video disconnect is expected and must still release its encoder.
        let _ = result;

        Ok(())
    }

    pub(super) fn control(
        &self,
        stream: &mut TcpStream,
        request: &super::serve::Request,
        token: &str,
    ) -> Result<()> {
        socket::control(self, stream, request, token)
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        self.close();
    }
}

fn fields(value: &Value, keys: &[&str]) -> Value {
    keys.iter()
        .map(|key| ((*key).to_owned(), value[*key].clone()))
        .collect::<serde_json::Map<_, _>>()
        .into()
}

fn token() -> String {
    use base64::Engine;
    let mut bytes = [0; 18];

    bytes[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(&uuid::Uuid::new_v4().as_bytes()[..2]);

    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(all(test, unix))]
mod tests;
