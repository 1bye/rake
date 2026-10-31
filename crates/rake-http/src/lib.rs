//! Rake-owned HTTP types over native HTTP and browser Fetch.
//!
//! Native I/O is adapted to the caller's executor. Browser futures remain local.
//! Cancellation and total request deadlines also cover response-body consumption.
//! Browser CORS, forbidden headers and credential policy still apply.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use event_listener::Event;
use futures::future::{Either, select};
use futures::{Stream, StreamExt};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use tracing::Instrument;
use url::Url;
use web_time::Instant;

#[cfg(feature = "devtools")]
mod inspection;

use rake_diagnostics::Collector;

/// A portable HTTP result.
pub type Result<T> = std::result::Result<T, Error>;

/// Inspectable failures without transport-specific types in public signatures.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A base URL, method, header or request value is invalid.
    #[error("invalid HTTP configuration or request: {0}")]
    Configuration(String),
    /// Relative endpoints need an explicit base on both platforms.
    #[error("a relative HTTP endpoint requires an explicitly configured base URL")]
    MissingBaseUrl,
    /// The request's cancellation token was cancelled.
    #[error("HTTP request cancelled")]
    Cancelled,
    /// The total request deadline elapsed, subject to host scheduling.
    #[error("HTTP request timed out")]
    Timeout,
    /// Network rejection. Browsers may not disclose the underlying reason.
    #[error("HTTP transport failed: {0}")]
    Transport(String),
    /// A success-only helper received a non-2xx response.
    #[error("HTTP response status {status}")]
    Status {
        /// Received HTTP status code.
        status: u16,
    },
    /// The response stream failed after headers arrived.
    #[error("HTTP response body failed: {0}")]
    Body(String),
    /// JSON serialization failed before transmission.
    #[error("HTTP JSON encoding failed: {0}")]
    Encode(#[source] serde_json::Error),
    /// The response body was not valid JSON for the requested type.
    #[error("HTTP JSON decoding failed: {0}")]
    Decode(#[source] serde_json::Error),
}

/// Shared request policy. Cloning copies configuration, not a transport.
#[derive(Clone, Default)]
#[must_use]
pub struct HttpConfig {
    base_url: Option<String>,
    timeout: Option<Duration>,
}

impl HttpConfig {
    /// Uses no base URL and no deadline.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets an absolute HTTP(S) base. Validated when constructing the client.
    ///
    /// URL joining follows URL rules: `items` keeps a base path, while `/items`
    /// replaces it. Relative requests never implicitly use the browser location.
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());

        self
    }

    /// Sets a total deadline spanning request start through response-body completion.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);

        self
    }
}

#[derive(Default)]
struct Cancellation {
    cancelled: AtomicBool,
    event: Event,
}

/// A shared cancellation signal. Cancelling does not undo server-side effects.
#[derive(Clone, Default)]
pub struct CancellationToken(Arc<Cancellation>);

impl CancellationToken {
    /// Creates an uncancelled token.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Wakes all requests using this token. Repeated cancellation is harmless.
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::Release);
        self.0.event.notify(usize::MAX);
    }

    /// Whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }

    /// Waits until cancellation is requested, including cancellation before this call.
    pub async fn cancelled(&self) {
        loop {
            let listener = self.0.event.listen();

            if self.is_cancelled() {
                return;
            }

            listener.await;
        }
    }
}

struct Client {
    transport: reqwest::Client,
    base_url: Option<Url>,
    timeout: Option<Duration>,
    collector: Collector,
}

/// An inexpensive cloneable handle to one configured transport and collector.
#[derive(Clone)]
pub struct HttpClient(Arc<Client>);

impl HttpClient {
    /// Creates a standalone client with bounded diagnostics and executor adaptation.
    ///
    /// # Errors
    /// Returns [`Error::Configuration`] for an invalid base URL or transport setup failure.
    pub fn new(config: HttpConfig) -> Result<Self> {
        Self::with_collector(config, Collector::default())
    }

    /// Creates a client contributing request lifecycle events to the given collector.
    ///
    /// # Errors
    /// Returns [`Error::Configuration`] for an invalid base URL or transport setup failure.
    pub fn with_collector(config: HttpConfig, collector: Collector) -> Result<Self> {
        let base_url = config.base_url.as_deref().map(parse_url).transpose()?;

        let transport = reqwest::Client::builder()
            .build()
            .map_err(|error| Error::Configuration(error.without_url().to_string()))?;

        Ok(Self(Arc::new(Client {
            transport,
            base_url,
            timeout: config.timeout,
            collector,
        })))
    }

    /// Reports whether two handles share the same configured transport.
    ///
    /// Cloning an [`HttpClient`] preserves the transport identity. Methods that
    /// intentionally rebuild transport policy, such as `without_redirects`, return
    /// a distinct transport.
    #[must_use]
    pub fn shares_transport_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Builds an equivalent native client that never follows redirects.
    ///
    /// # Errors
    /// Returns [`Error::Configuration`] if the native transport cannot be constructed.
    #[cfg(not(target_family = "wasm"))]
    pub fn without_redirects(&self) -> Result<Self> {
        let transport = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| Error::Configuration(error.without_url().to_string()))?;

        Ok(Self(Arc::new(Client {
            transport,
            base_url: self.0.base_url.clone(),
            timeout: self.0.timeout,
            collector: self.0.collector.clone(),
        })))
    }

    /// Begins building a GET request. Network work starts when `send` is polled.
    #[cfg_attr(feature = "devtools", track_caller)]
    pub fn get(&self, endpoint: &str) -> Request {
        self.request("GET", endpoint)
    }

    /// Begins building a POST request.
    #[cfg_attr(feature = "devtools", track_caller)]
    pub fn post(&self, endpoint: &str) -> Request {
        self.request("POST", endpoint)
    }

    /// Builds any valid HTTP method, including PUT, PATCH, DELETE and HEAD.
    #[cfg_attr(feature = "devtools", track_caller)]
    pub fn request(&self, method: &str, endpoint: &str) -> Request {
        let builder = self.resolve(endpoint).and_then(|url| {
            let method = reqwest::Method::from_bytes(method.as_bytes())
                .map_err(|error| Error::Configuration(error.to_string()))?;

            Ok(self.0.transport.request(method, url))
        });

        Request {
            client: self.clone(),
            builder,
            cancellation: CancellationToken::new(),
            timeout: self.0.timeout,
            #[cfg(feature = "devtools")]
            source: std::panic::Location::caller(),
        }
    }

    fn resolve(&self, endpoint: &str) -> Result<Url> {
        match Url::parse(endpoint) {
            Ok(url) => validate_url(url),
            Err(url::ParseError::RelativeUrlWithoutBase) => {
                let base = self.0.base_url.as_ref().ok_or(Error::MissingBaseUrl)?;

                let url = base
                    .join(endpoint)
                    .map_err(|error| Error::Configuration(error.to_string()))?;

                validate_url(url)
            }
            Err(error) => Err(Error::Configuration(error.to_string())),
        }
    }
}

fn parse_url(value: &str) -> Result<Url> {
    let url = Url::parse(value).map_err(|error| Error::Configuration(error.to_string()))?;

    validate_url(url)
}

fn validate_url(url: Url) -> Result<Url> {
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(Error::Configuration(
            "an absolute HTTP(S) URL is required".into(),
        ));
    }

    Ok(url)
}

/// An owned request builder. Validation errors are returned by `send`.
#[must_use = "requests run only when send or send_json is awaited"]
pub struct Request {
    client: HttpClient,
    builder: Result<reqwest::RequestBuilder>,
    cancellation: CancellationToken,
    timeout: Option<Duration>,
    #[cfg(feature = "devtools")]
    source: &'static std::panic::Location<'static>,
}

impl Request {
    /// Appends URL-encoded query parameters using standard Serde serialization.
    pub fn query<T: Serialize + ?Sized>(mut self, query: &T) -> Self {
        self.builder = self.builder.map(|builder| builder.query(query));

        self
    }

    /// Adds a validated header. Browser forbidden-header rules still apply.
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.builder = self.builder.map(|builder| builder.header(name, value));

        self
    }

    /// Sets an owned binary body.
    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.builder = self.builder.map(|builder| builder.body(body.into()));

        self
    }

    /// Serializes a JSON body before transmission and sets its content type.
    pub fn json<T: Serialize + ?Sized>(mut self, value: &T) -> Self {
        self.builder = self.builder.and_then(|builder| {
            let body = serde_json::to_vec(value).map_err(Error::Encode)?;

            Ok(builder
                .header("content-type", "application/json")
                .body(body))
        });

        self
    }

    /// Associates an explicit cancellation token with request and body consumption.
    pub fn cancellation(mut self, token: CancellationToken) -> Self {
        self.cancellation = token;

        self
    }

    /// Overrides the client's total request deadline.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);

        self
    }

    /// Sends the request, returning headers and an unconsumed body, including non-2xx.
    ///
    /// # Errors
    /// Returns configuration/encoding errors before transmission, [`Error::Cancelled`]
    /// or [`Error::Timeout`] when interrupted, or [`Error::Transport`] if sending fails.
    pub async fn send(self) -> Result<Response> {
        let request = self
            .builder?
            .build()
            .map_err(|error| Error::Configuration(error.without_url().to_string()))?;

        let mut url = request.url().clone();
        let _ = url.set_username("");
        let _ = url.set_password(None);

        url.set_fragment(None);
        #[cfg(not(feature = "devtools"))]
        url.set_query(None);
        #[cfg(feature = "devtools")]
        let url = inspection::url(&url);
        let deadline = self
            .timeout
            .map(|timeout| {
                Instant::now().checked_add(timeout).ok_or_else(|| {
                    Error::Configuration("HTTP timeout exceeds the host clock range".into())
                })
            })
            .transpose()?;

        let mut lifecycle = Lifecycle::new(self.client.0.collector.clone());

        let span = tracing::info_span!("http.request", request_id = lifecycle.id);

        // A filtered request span must not erase its parent's captured causal context.
        if !span.is_disabled() {
            lifecycle.context = lifecycle.collector.span_context(&span);
        }

        #[cfg(feature = "devtools")]
        {
            lifecycle.context.set_source(rake_diagnostics::Source {
                file: self.source.file().into(),
                line: self.source.line(),
            });
        }
        #[cfg(feature = "devtools")]
        lifecycle.collector.begin_http(
            lifecycle.id,
            request
                .body()
                .and_then(reqwest::Body::as_bytes)
                .unwrap_or_default(),
        );
        lifecycle.event(
            "started",
            [
                ("method", request.method().as_str().into()),
                ("url", url.as_str().into()),
                #[cfg(feature = "devtools")]
                ("request_headers", inspection::headers(request.headers())),
                #[cfg(feature = "devtools")]
                ("has_body", request.body().is_some().into()),
            ],
        );
        let scope = Scope {
            token: self.cancellation,
            deadline,
        };

        let result = scope
            .run(compatible(async {
                self.client.0.transport.execute(request).await
            }))
            .instrument(span.clone())
            .await;

        let response = match result {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                lifecycle.finish("failed");

                return Err(Error::Transport(error.without_url().to_string()));
            }
            Err(error) => {
                lifecycle.finish(error_phase(&error));

                return Err(error);
            }
        };

        let status = response.status().as_u16();
        let headers = response.headers().clone();

        lifecycle.event(
            "headers_received",
            [
                ("status", status.into()),
                #[cfg(feature = "devtools")]
                ("response_headers", inspection::headers(&headers)),
                #[cfg(feature = "devtools")]
                ("response_url", inspection::url(response.url()).into()),
                #[cfg(feature = "devtools")]
                (
                    "status_text",
                    response
                        .status()
                        .canonical_reason()
                        .unwrap_or_default()
                        .into(),
                ),
                #[cfg(all(feature = "devtools", not(target_family = "wasm")))]
                (
                    "protocol",
                    format!("{:?}", response.version())
                        .to_ascii_lowercase()
                        .into(),
                ),
            ],
        );

        Ok(Response {
            status,
            headers,
            body: Box::pin(response.bytes_stream().map(|chunk| {
                chunk
                    .map(|bytes| bytes.to_vec())
                    .map_err(|error| Error::Body(error.without_url().to_string()))
            })),
            scope,
            lifecycle,
            span,
        })
    }

    /// Requires a 2xx status and decodes the JSON response body.
    ///
    /// # Errors
    /// Returns the failures from [`Self::send`], [`Error::Status`] for non-2xx
    /// responses, body-consumption failures, or [`Error::Decode`] for invalid JSON.
    pub async fn send_json<T: DeserializeOwned>(self) -> Result<T> {
        self.send().await?.error_for_status()?.json().await
    }
}

#[cfg(not(target_family = "wasm"))]
type Body = Pin<Box<dyn Stream<Item = Result<Vec<u8>>> + Send + 'static>>;
#[cfg(target_family = "wasm")]
type Body = Pin<Box<dyn Stream<Item = Result<Vec<u8>>> + 'static>>;

/// Response metadata and a lazily consumed body. Dropping it reports abandonment.
pub struct Response {
    status: u16,
    headers: reqwest::header::HeaderMap,
    body: Body,
    scope: Scope,
    lifecycle: Lifecycle,
    span: tracing::Span,
}

impl Response {
    /// The received HTTP status code.
    #[must_use]
    pub fn status(&self) -> u16 {
        self.status
    }

    /// Visible response headers, including repeated values. Browser filtering applies.
    pub fn headers(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_bytes()))
    }

    /// Returns this response for 2xx, or an inspectable status error otherwise.
    ///
    /// # Errors
    /// Returns [`Error::Status`] when the received status is outside 200–299.
    pub fn error_for_status(self) -> Result<Self> {
        if !(200..300).contains(&self.status) {
            return Err(Error::Status {
                status: self.status,
            });
        }

        Ok(self)
    }

    /// Reads one chunk. `None` marks completion; errors terminate consumption.
    /// Development capture retains at most 64 KiB when `devtools` is enabled; it never reads ahead.
    ///
    /// # Errors
    /// Returns [`Error::Cancelled`], [`Error::Timeout`] or [`Error::Body`]. After
    /// any terminal error, subsequent calls return `Ok(None)`.
    pub async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>> {
        if self.lifecycle.finished {
            return Ok(None);
        }

        let result = self
            .scope
            .run(compatible(self.body.next()))
            .instrument(self.span.clone())
            .await;

        match result {
            Ok(Some(Ok(bytes))) => {
                #[cfg(feature = "devtools")]
                {
                    self.lifecycle.capture_lost |= !self
                        .lifecycle
                        .collector
                        .append_http(self.lifecycle.id, &bytes);
                }
                self.lifecycle
                    .event("body_progress", [("bytes", (bytes.len() as u64).into())]);

                Ok(Some(bytes))
            }
            Ok(None) => {
                self.lifecycle.finish("completed");

                Ok(None)
            }
            Ok(Some(Err(error))) | Err(error) => {
                self.lifecycle.finish(error_phase(&error));
                self.body = Box::pin(futures::stream::empty());

                Err(error)
            }
        }
    }

    /// Explicitly buffers the response body into owned bytes.
    ///
    /// # Errors
    /// Returns the cancellation, timeout and body failures from [`Self::next_chunk`].
    pub async fn bytes(mut self) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();

        while let Some(chunk) = self.next_chunk().await? {
            bytes.extend(chunk);
        }

        Ok(bytes)
    }

    /// Reads and decodes JSON. Does not enforce status; use `error_for_status` first.
    ///
    /// # Errors
    /// Returns body-consumption failures or [`Error::Decode`] if the body is not
    /// valid JSON for `T`. HTTP status validation remains the caller’s responsibility.
    pub async fn json<T: DeserializeOwned>(self) -> Result<T> {
        serde_json::from_slice(&self.bytes().await?).map_err(Error::Decode)
    }
}

struct Scope {
    token: CancellationToken,
    deadline: Option<Instant>,
}

impl Scope {
    async fn run<T>(&self, work: impl Future<Output = T>) -> Result<T> {
        if self.token.is_cancelled() {
            return Err(Error::Cancelled);
        }

        if self
            .deadline
            .is_some_and(|deadline| deadline <= Instant::now())
        {
            return Err(Error::Timeout);
        }

        let stop = async {
            let deadline = async {
                match self.deadline {
                    Some(deadline) => {
                        sleep(deadline.saturating_duration_since(Instant::now())).await
                    }
                    None => futures::future::pending().await,
                }
            };

            match select(Box::pin(self.token.cancelled()), Box::pin(deadline)).await {
                Either::Left(_) => Error::Cancelled,
                Either::Right(_) => Error::Timeout,
            }
        };

        match select(Box::pin(work), Box::pin(stop)).await {
            Either::Left((value, _)) => Ok(value),
            Either::Right((error, _)) => Err(error),
        }
    }
}

async fn compatible<T>(future: impl Future<Output = T>) -> T {
    #[cfg(not(target_family = "wasm"))]
    {
        async_compat::Compat::new(future).await
    }

    #[cfg(target_family = "wasm")]
    {
        future.await
    }
}

/// Waits on the host timer without requiring a particular executor.
/// Durations beyond the host clock's range remain pending until dropped.
pub async fn sleep(duration: Duration) {
    let Some(deadline) = Instant::now().checked_add(duration) else {
        return futures::future::pending().await;
    };

    #[cfg(not(target_family = "wasm"))]
    async_io::Timer::at(deadline).await;

    #[cfg(target_family = "wasm")]
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());

        if remaining.is_zero() {
            break;
        }

        let milliseconds = remaining.as_millis().clamp(1, i32::MAX as u128) as u32;

        gloo_timers::future::TimeoutFuture::new(milliseconds).await;
    }
}

fn error_phase(error: &Error) -> &'static str {
    match error {
        Error::Cancelled => "cancelled",
        Error::Timeout => "timed_out",
        _ => "failed",
    }
}

struct Lifecycle {
    id: u64,
    collector: Collector,
    finished: bool,
    context: rake_diagnostics::CapturedContext,
    started: Instant,
    bytes: u64,
    metadata: BTreeMap<String, Value>,
    #[cfg(feature = "devtools")]
    capture_lost: bool,
}

impl Lifecycle {
    fn new(collector: Collector) -> Self {
        static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

        #[cfg(feature = "devtools")]
        let start_us = collector.elapsed_us();
        Self {
            id: NEXT_REQUEST.fetch_add(1, Ordering::Relaxed),
            context: collector.context(),
            collector,
            finished: false,
            started: Instant::now(),
            bytes: 0,
            metadata: {
                #[cfg(feature = "devtools")]
                {
                    BTreeMap::from([("request_start_us".into(), start_us.into())])
                }
                #[cfg(not(feature = "devtools"))]
                {
                    BTreeMap::new()
                }
            },
            #[cfg(feature = "devtools")]
            capture_lost: false,
        }
    }

    fn event<const N: usize>(&mut self, phase: &str, fields: [(&str, Value); N]) {
        let mut fields: BTreeMap<String, Value> = fields
            .into_iter()
            .map(|(key, value)| (key.into(), value))
            .collect();

        for key in ["method", "url", "status"] {
            if let Some(value) = fields.get(key) {
                self.metadata.insert(key.into(), value.clone());
            }
        }

        if let Some(bytes) = fields.get("bytes").and_then(Value::as_u64) {
            self.bytes = self.bytes.saturating_add(bytes);
        }

        fields.extend(self.metadata.clone());
        fields.insert("received_bytes".into(), self.bytes.into());
        fields.insert(
            "duration_us".into(),
            (self.started.elapsed().as_micros().min(u64::MAX as u128) as u64).into(),
        );
        fields.insert("request_id".into(), self.id.into());
        fields.insert("phase".into(), phase.into());

        self.collector.record_context(
            self.context.clone(),
            "INFO",
            "rake::http",
            "request",
            Vec::new(),
            fields,
            None,
        );
    }

    fn finish(&mut self, phase: &str) {
        self.finished = true;
        #[cfg(feature = "devtools")]
        self.collector.finish_http(
            self.id,
            if self.capture_lost {
                "capture_lost"
            } else {
                phase
            },
        );
        self.event(phase, []);
    }
}

impl Drop for Lifecycle {
    fn drop(&mut self) {
        if !self.finished {
            self.finish("abandoned");
        }
    }
}

// Optional facade integration keeps the protocol independent of HTTP.
#[cfg(feature = "task-errors")]
impl From<Error> for rake_protocol::tasks::TaskError {
    fn from(error: Error) -> Self {
        Self::Failed(error.to_string())
    }
}
