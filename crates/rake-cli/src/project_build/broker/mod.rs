//! Authenticated runtime forwarding and browser rendezvous owned by the dev origin.
mod bridge;
mod source;

use bridge::Bridge;
use rake_http::{CancellationToken, HttpClient, HttpConfig};
use serde_json::{Value, json};

use std::{
    fmt, fs,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::{
    fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
    net::UnixStream,
};

pub(super) const MAX_BYTES: usize = 1024 * 1024;

type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub(super) struct Error {
    pub(super) status: u16,
    message: String,
    optional_discovery: bool,
}

impl Error {
    pub(super) fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            optional_discovery: false,
        }
    }

    fn transport(message: impl Into<String>) -> Self {
        Self {
            status: 502,
            message: message.into(),
            optional_discovery: true,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(f)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::new(502, error.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::new(400, error.to_string())
    }
}

pub(super) struct Options {
    pub(super) url: String,
    pub(super) directory: PathBuf,
    pub(super) development: Option<PathBuf>,
    pub(super) output: Option<PathBuf>,
    pub(super) allow_native: bool,
    pub(super) cdp_url: Option<String>,
    pub(super) source_root: PathBuf,
    pub(super) cli: PathBuf,
}

pub(super) struct Broker {
    bridge: Bridge,
    options: Options,
    http: HttpClient,
    cancelled: CancellationToken,
}

impl Broker {
    pub(super) fn new(options: Options) -> Result<Self> {
        Ok(Self {
            bridge: Bridge::new(
                options.url.clone(),
                options.directory.clone(),
                options.development.clone(),
            )?,
            options,
            http: HttpClient::new(HttpConfig::new())
                .and_then(|client| client.without_redirects())
                .map_err(|error| Error::new(502, error.to_string()))?,
            cancelled: CancellationToken::new(),
        })
    }

    pub(super) fn token(&self) -> &str {
        &self.bridge.token
    }

    pub(super) fn authenticate(&self, supplied: &str) -> bool {
        use subtle::ConstantTimeEq;

        bool::from(self.bridge.token.as_bytes().ct_eq(supplied.as_bytes()))
    }

    pub(super) fn close(&self) {
        self.cancelled.cancel();
        self.bridge.close();
    }

    fn manifest(&self, runtime: &Value) -> Result<Value> {
        let unavailable = || Error::new(404, "runtime unavailable; select a live instance");

        let runtime = runtime
            .as_str()
            .filter(|id| uuid::Uuid::parse_str(id).is_ok_and(|parsed| parsed.to_string() == *id))
            .ok_or_else(unavailable)?;

        // Private-file violations retain their security status, as in the Python broker.
        let value = match private_read(&self.bridge.directory.join(format!("{runtime}.json"))) {
            Err(error) if error.status == 502 || error.status == 400 => return Err(unavailable()),
            result => result?,
        };

        let endpoint = &value["endpoint"];

        let valid = match endpoint["type"].as_str() {
            Some("local") => endpoint["path"].is_string(),
            Some("web") => endpoint["url"].is_string() && endpoint["token"].is_string(),
            _ => false,
        };

        if value["info"]["runtime_id"] != runtime
            || !value["application_id"].is_string()
            || !value["pid"].is_i64()
            || !valid
        {
            return Err(unavailable());
        }

        Ok(value)
    }

    fn instances(&self) -> Result<Value> {
        let mut paths = fs::read_dir(&self.bridge.directory)?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect::<Vec<_>>();

        paths.sort();
        let development = self.bridge.development();
        let mut values = Vec::new();

        for path in paths.into_iter().take(128) {
            let Some(runtime) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };

            let Ok(item) = self.manifest(&Value::from(runtime)) else {
                continue;
            };

            let local = item["endpoint"]["type"] == "local";

            if local && !self.options.allow_native {
                continue;
            }

            let reload = if local && development["runtime_id"] == runtime {
                development["reload"].clone()
            } else {
                self.bridge.reload(runtime)
            };

            values.push(json!({"application_id":item["application_id"],"pid":item["pid"],"info":item["info"],"transport":item["endpoint"]["type"],"restored_from":reload["restored_from"],"reload_phase":reload["phase"]}));
        }

        Ok(values.into())
    }

    pub(super) fn handle(&self, action: &str, body: &Value) -> Result<Value> {
        match action {
            "inspect_instances" => self.instances(),
            "inspect_output" => {
                let after = body
                    .get("after")
                    .unwrap_or(&Value::from(0))
                    .as_u64()
                    .ok_or_else(|| Error::new(400, "invalid output cursor"))?;

                let mut snapshot = match self.options.output.as_ref().filter(|path| path.is_file())
                {
                    Some(path) => private_read(path)?,
                    None => json!({"records":[],"dropped":0}),
                };

                let records = snapshot
                    .get_mut("records")
                    .and_then(Value::as_array_mut)
                    .ok_or_else(|| Error::new(400, "invalid process output records"))?;

                records.retain(|record| {
                    record["sequence"]
                        .as_u64()
                        .is_some_and(|sequence| sequence > after)
                });

                Ok(snapshot)
            }
            "inspect_source" => {
                let manifest = self.manifest(&body["runtime_id"])?;

                if manifest["endpoint"]["type"] == "local" && !self.options.allow_native {
                    return Err(Error::new(
                        403,
                        "Web application origins cannot open native runtime source",
                    ));
                }

                let args = source::arguments(body)?;

                source::open(
                    &self.options,
                    body["runtime_id"].as_str().expect("validated runtime"),
                    &args,
                    &self.cancelled,
                )
            }
            "inspect_cdp" => {
                let Some(url) = &self.options.cdp_url else {
                    return Ok(json!([]));
                };

                endpoint_url(url, true)?;

                match self.http_json(
                    url,
                    "/json/list",
                    None,
                    Duration::from_secs(3),
                    "CDP discovery exceeds 1 MiB",
                ) {
                    Err(error) if error.optional_discovery || error.status == 400 => Ok(json!([])),
                    result => result,
                }
            }
            "inspect" => self.inspect(body),
            action if action.starts_with("inspect") => {
                Err(Error::new(404, "unknown inspector endpoint"))
            }
            _ => self.bridge.handle(action, body),
        }
    }

    fn inspect(&self, body: &Value) -> Result<Value> {
        let request = &body["request"];

        if !request.is_object() || request["protocol"] != 1 {
            return Err(Error::new(400, "invalid runtime request"));
        }

        let manifest = self.manifest(&request["runtime_id"])?;
        let runtime = request["runtime_id"].as_str().expect("validated runtime");
        let endpoint = &manifest["endpoint"];

        if endpoint["type"] == "web" {
            let body = json!({"runtime_id":runtime,"request":request});

            if self.bridge.contains(runtime) {
                return self.bridge.handle("client", &body);
            }

            let url = endpoint["url"].as_str().expect("validated endpoint");

            endpoint_url(url, false)?;

            return self.http_json(
                url,
                "/__rake/client",
                Some((&body, endpoint["token"].as_str().expect("validated token"))),
                Duration::from_secs(12),
                "runtime response too large",
            );
        }

        if !self.options.allow_native {
            return Err(Error::new(
                403,
                "application Web origins cannot access native IPC; use rake inspect",
            ));
        }

        #[cfg(unix)]
        let path = self.bridge.directory.join(format!("{runtime}.sock"));
        #[cfg(windows)]
        let path = rake_client::pipe::path(&runtime);

        if Path::new(endpoint["path"].as_str().expect("validated endpoint")) != path {
            return Err(Error::new(
                403,
                "native endpoint must belong to the private control directory",
            ));
        }

        #[cfg(unix)]
        let metadata = fs::symlink_metadata(&path)?;

        #[cfg(unix)]
        if !metadata.file_type().is_socket() || !owned_private(&metadata) {
            return Err(Error::new(
                403,
                "native endpoint must be an owned private socket",
            ));
        }

        let mut bytes = serde_json::to_vec(request)?;

        bytes.push(b'\n');

        if bytes.len() > MAX_BYTES {
            return Err(Error::new(413, "request frame too large"));
        }

        futures::executor::block_on(async {
            use futures::io::{AsyncReadExt, AsyncWriteExt};
            #[cfg(unix)]
            let mut connection = self
                .socket_io(async_io::Async::<UnixStream>::connect(path))
                .await?;
            #[cfg(windows)]
            let mut connection = self
                .socket_io(rake_client::pipe::Stream::connect(&path))
                .await?;

            self.socket_io(connection.write_all(&bytes)).await?;
            let mut data = Vec::new();
            let mut buffer = [0; 65536];

            loop {
                let count = (MAX_BYTES + 1 - data.len()).min(buffer.len());

                let count = self
                    .socket_io(connection.read(&mut buffer[..count]))
                    .await?;

                if count == 0 {
                    return Err(Error::new(
                        502,
                        "runtime disconnected; delivery may have occurred",
                    ));
                }

                data.extend_from_slice(&buffer[..count]);

                if data.len() > MAX_BYTES {
                    return Err(Error::new(413, "runtime response too large"));
                }

                if data.contains(&b'\n') {
                    return Ok(serde_json::from_slice(&data)?);
                }
            }
        })
    }

    async fn socket_io<T>(
        &self,
        work: impl std::future::Future<Output = std::io::Result<T>>,
    ) -> Result<T> {
        use futures::future::{Either, select};
        let stop = async {
            match select(
                Box::pin(self.cancelled.cancelled()),
                Box::pin(async_io::Timer::after(Duration::from_secs(5))),
            )
            .await
            {
                Either::Left(_) => Error::new(503, "Development bridge stopped"),
                Either::Right(_) => Error::new(502, "runtime exchange timed out"),
            }
        };

        // Poll cancellation/deadline first, including while connecting to a full backlog.
        match select(Box::pin(stop), Box::pin(work)).await {
            Either::Left((error, _)) => Err(error),
            Either::Right((result, _)) => Ok(result?),
        }
    }

    fn http_json(
        &self,
        url: &str,
        path: &str,
        body: Option<(&Value, &str)>,
        timeout: Duration,
        oversized: &str,
    ) -> Result<Value> {
        futures::executor::block_on(async {
            let url = format!("{}{path}", url.trim_end_matches('/'));

            let request = if let Some((body, token)) = body {
                self.http
                    .post(&url)
                    .header("X-Rake-Token", token)
                    .json(body)
            } else {
                self.http.get(&url)
            };

            let mut response = request
                .timeout(timeout)
                .cancellation(self.cancelled.clone())
                .send()
                .await
                .map_err(|error| Error::transport(error.to_string()))?;

            if (300..400).contains(&response.status()) {
                return Err(Error::new(502, "development endpoints cannot redirect"));
            }

            if !(200..300).contains(&response.status()) {
                return Err(Error::transport(format!(
                    "HTTP Error {}",
                    response.status()
                )));
            }

            let mut data = Vec::new();

            while let Some(chunk) = response
                .next_chunk()
                .await
                .map_err(|error| Error::transport(error.to_string()))?
            {
                if data.len() + chunk.len() > MAX_BYTES {
                    return Err(Error::new(413, oversized));
                }

                data.extend_from_slice(&chunk);
            }

            Ok(serde_json::from_slice(&data)?)
        })
    }
}

impl Drop for Broker {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(unix)]
fn owned_private(metadata: &fs::Metadata) -> bool {
    // SAFETY: geteuid has no pointer or thread preconditions.
    metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o077 == 0
}

fn private_read(path: &Path) -> Result<Value> {
    #[cfg(unix)]
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;

    #[cfg(windows)]
    let file = rake_client::private::open(path).map_err(|error| {
        Error::new(
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                403
            } else {
                502
            },
            error.to_string(),
        )
    })?;

    #[cfg(unix)]
    let metadata = file.metadata()?;

    #[cfg(unix)]
    if !metadata.is_file() || !owned_private(&metadata) {
        return Err(Error::new(
            403,
            "discovery files must be owned private regular files",
        ));
    }

    let mut bytes = Vec::new();

    file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes)?;

    if bytes.len() > MAX_BYTES {
        return Err(Error::new(413, "discovery frame too large"));
    }

    Ok(serde_json::from_slice(&bytes)?)
}

fn endpoint_url(raw: &str, cdp: bool) -> Result<url::Url> {
    let message = if cdp {
        "CDP discovery must use IPv4 loopback HTTP"
    } else {
        "Web control endpoint must be loopback HTTP"
    };

    // Check the original authority too: Url normalizes numeric aliases and default ports.
    let authority = raw
        .strip_prefix("http://")
        .and_then(|tail| tail.split(['/', '?', '#']).next())
        .unwrap_or_default();

    let (host, port) = authority
        .rsplit_once(':')
        .ok_or_else(|| Error::new(403, message))?;

    let parsed = url::Url::parse(raw).map_err(|_| Error::new(403, message))?;

    if !matches!(host, "127.0.0.1" | "localhost")
        || (cdp && host != "127.0.0.1")
        || port.parse::<u16>().ok().is_none_or(|port| port == 0)
        || !matches!(
            raw.strip_prefix("http://")
                .and_then(|tail| tail.strip_prefix(authority)),
            Some("" | "/")
        )
        || !matches!(parsed.path(), "" | "/")
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(Error::new(403, message));
    }

    Ok(parsed)
}

fn token() -> String {
    use base64::Engine;
    let mut bytes = [0; 32];

    bytes[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());

    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(all(test, unix))]
use bridge::Ticket;
#[cfg(all(test, unix))]
use std::time::Instant;

#[cfg(all(test, unix))]
mod tests;
