//! Bounded structured diagnostics, independent of authoritative application state.
//!
//! This crate owns record retention, tracing context and nonblocking output. It has
//! no router, HTTP client or UI dependency. The `rake` facade re-exports these types
//! through `rake::diagnostics`.
//!
//! ```
//! use rake_diagnostics::Collector;
//! use tracing_subscriber::prelude::*;
//!
//! let collector = Collector::new(128);
//! let subscriber = tracing_subscriber::registry().with(collector.layer());
//! tracing::subscriber::with_default(subscriber, || {
//!     tracing::info!(task = "export", "started");
//! });
//! let snapshot = collector.snapshot_after(0, Some("event"));
//! assert_eq!(snapshot.records.len(), 1);
//! assert_eq!(snapshot.records[0].fields["task"], "export");
//! ```

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record as SpanRecord};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::{LookupSpan, SpanRef};
use tracing_subscriber::{Layer, Registry};
use web_time::Instant;

/// A location reported by tracing metadata; never inferred from span names.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Source {
    /// Rust source path, relative to the build workspace when available.
    pub file: String,
    /// One-based source line.
    pub line: u32,
}

/// One owned diagnostic record. Values are retained as typed JSON where available.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    /// Monotonic sequence within this collector; not a router revision.
    pub sequence: u64,
    /// Emitting tracing call site; absent for older or manually recorded events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    /// Time since collector creation, in microseconds.
    pub elapsed_us: u64,
    /// Tracing severity or HTTP lifecycle severity.
    pub level: String,
    /// Emitting module or framework subsystem.
    pub target: String,
    /// `event`, `span_open`, `span_close`, or `request`.
    pub kind: String,
    /// Enclosing span names from root to leaf.
    pub spans: Vec<String>,
    /// Collector-local span identity; absent when no instrumented span is active.
    #[serde(default)]
    pub span_id: Option<u64>,
    /// Parent identity, independent of potentially identical span names.
    #[serde(default)]
    pub parent_span_id: Option<u64>,
    /// Elapsed wall time on span close, not CPU usage.
    #[serde(default)]
    pub duration_us: Option<u64>,
    /// Structured fields; debug/display fields necessarily contain formatted text.
    pub fields: BTreeMap<String, Value>,
}

/// An owned view of the bounded collector, including observable data loss.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    /// Retained records in sequence order.
    pub records: Vec<Record>,
    /// Records lost to capacity or writer contention since collector creation.
    pub dropped: u64,
    /// Terminal output messages lost to the bounded native writer queue.
    pub output_dropped: u64,
}

struct Buffer {
    records: VecDeque<Record>,
    sequence: u64,
}

struct Inner {
    buffer: Mutex<Buffer>,
    capacity: usize,
    dropped: AtomicU64,
    output_dropped: AtomicU64,
    started: Instant,
    next_span: AtomicU64,
}

/// A cheap shared handle to bounded records. Producers never wait for its lock.
#[derive(Clone)]
pub struct Collector(Arc<Inner>);

impl Default for Collector {
    fn default() -> Self {
        Self::new(1024)
    }
}

impl Collector {
    /// Creates a collector. Capacity zero records only loss counts.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self(Arc::new(Inner {
            buffer: Mutex::new(Buffer {
                records: VecDeque::new(),
                sequence: 0,
            }),
            capacity,
            dropped: AtomicU64::new(0),
            output_dropped: AtomicU64::new(0),
            started: Instant::now(),
            next_span: AtomicU64::new(1),
        }))
    }

    /// Copies retained records without draining the collector.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        self.snapshot_after(0, None)
    }

    /// Copies matching records after an exclusive sequence cursor, in sequence order.
    ///
    /// `None` selects all kinds; `Some("span")` selects both span-open and span-close
    /// records. Other kinds match exactly. Filtering precedes cloning, so incremental
    /// polling copies only the selected records. Loss counters always cover the
    /// collector's entire lifetime, even when this snapshot contains no records.
    #[must_use]
    pub fn snapshot_after(&self, after: u64, kind: Option<&str>) -> Snapshot {
        let buffer = self
            .0
            .buffer
            .lock()
            .unwrap_or_else(|error| error.into_inner());

        Snapshot {
            records: buffer
                .records
                .iter()
                .filter(|record| {
                    record.sequence > after
                        && kind.is_none_or(|kind| {
                            if kind == "span" {
                                record.kind == "span_open" || record.kind == "span_close"
                            } else {
                                record.kind == kind
                            }
                        })
                })
                .cloned()
                .collect(),
            dropped: self.0.dropped.load(Ordering::Relaxed),
            output_dropped: self.0.output_dropped.load(Ordering::Relaxed),
        }
    }

    /// Supplies a layer for a host-owned tracing subscriber.
    #[must_use]
    pub fn layer(&self) -> CollectorLayer {
        CollectorLayer(self.clone())
    }

    /// Records an owned event using this collector's current tracing context.
    /// Writer contention or capacity overflow is counted rather than blocking producers.
    pub fn record(
        &self,
        level: &str,
        target: &str,
        kind: &str,
        spans: Vec<String>,
        fields: BTreeMap<String, Value>,
    ) {
        self.record_context(self.context(), level, target, kind, spans, fields, None);
    }

    /// Captures this collector's current tracing metadata for later asynchronous work.
    #[must_use]
    pub fn context(&self) -> CapturedContext {
        self.span_context(&tracing::Span::current())
    }

    /// Captures metadata from a span on a registry-backed subscriber carrying this layer.
    #[must_use]
    pub fn span_context(&self, span: &tracing::Span) -> CapturedContext {
        span.with_subscriber(|(id, dispatch)| {
            dispatch
                .downcast_ref::<Registry>()
                .and_then(|registry| registry.span(id))
                .map(|span| capture(span, self))
                .unwrap_or_default()
        })
        .unwrap_or_default()
    }

    #[allow(clippy::too_many_arguments)]
    /// Records an event with context captured by this collector, overlaid by explicit fields.
    /// Empty `spans` keeps the captured span path; `duration_us` is optional elapsed wall time.
    pub fn record_context(
        &self,
        mut context: CapturedContext,
        level: &str,
        target: &str,
        kind: &str,
        spans: Vec<String>,
        fields: BTreeMap<String, Value>,
        duration_us: Option<u64>,
    ) {
        context.fields.extend(fields);
        let spans = if spans.is_empty() {
            context.spans
        } else {
            spans
        };

        let Ok(mut buffer) = self.0.buffer.try_lock() else {
            self.0.dropped.fetch_add(1, Ordering::Relaxed);

            return;
        };

        if self.0.capacity == 0 {
            self.0.dropped.fetch_add(1, Ordering::Relaxed);

            return;
        }

        if buffer.records.len() == self.0.capacity {
            buffer.records.pop_front();
            self.0.dropped.fetch_add(1, Ordering::Relaxed);
        }

        buffer.sequence += 1;
        let sequence = buffer.sequence;

        buffer.records.push_back(Record {
            sequence,
            source: context.source,
            elapsed_us: self.0.started.elapsed().as_micros().min(u64::MAX as u128) as u64,
            level: level.into(),
            target: target.into(),
            kind: kind.into(),
            spans,
            fields: context.fields,
            span_id: context.span_id,
            parent_span_id: context.parent_span_id,
            duration_us,
        });
    }
}

#[derive(Default)]
struct Fields(BTreeMap<String, Value>);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.0
            .insert(field.name().into(), format!("{value:?}").into());
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().into(), value.into());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().into(), value.into());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0.insert(field.name().into(), value.into());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.0.insert(field.name().into(), value.into());
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.0.insert(field.name().into(), Value::from(value));
    }
}

/// A composable tracing layer backed by a [`Collector`].
pub struct CollectorLayer(Collector);

/// Owned causal metadata captured before asynchronous work changes context.
#[derive(Clone, Debug, Default)]
pub struct CapturedContext {
    source: Option<Source>,
    spans: Vec<String>,
    fields: BTreeMap<String, Value>,
    span_id: Option<u64>,
    parent_span_id: Option<u64>,
}

struct SpanData {
    id: u64,
    parent: Option<u64>,
    started: Instant,
    fields: BTreeMap<String, Value>,
}

#[derive(Default)]
struct SpanContexts(Vec<(std::sync::Weak<Inner>, SpanData)>);

impl SpanContexts {
    fn get(&self, collector: &Collector) -> Option<&SpanData> {
        let key = Arc::downgrade(&collector.0);

        self.0
            .iter()
            .find(|(owner, _)| owner.ptr_eq(&key))
            .map(|(_, data)| data)
    }

    fn get_mut(&mut self, collector: &Collector) -> Option<&mut SpanData> {
        let key = Arc::downgrade(&collector.0);

        self.0
            .iter_mut()
            .find(|(owner, _)| owner.ptr_eq(&key))
            .map(|(_, data)| data)
    }
}

fn capture<S: for<'lookup> LookupSpan<'lookup>>(
    span: SpanRef<'_, S>,
    collector: &Collector,
) -> CapturedContext {
    let mut context = CapturedContext::default();

    for span in span.scope().from_root() {
        context.spans.push(span.name().into());
        let extensions = span.extensions();

        if let Some(data) = extensions
            .get::<SpanContexts>()
            .and_then(|contexts| contexts.get(collector))
        {
            context.fields.extend(data.fields.clone());
            context.span_id = Some(data.id);
            context.parent_span_id = data.parent;
        }
    }

    context
}

impl<S: Subscriber + for<'lookup> LookupSpan<'lookup>> Layer<S> for CollectorLayer {
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, cx: Context<'_, S>) {
        let Some(span) = cx.span(id) else {
            return;
        };

        let parent = span.parent().and_then(|parent| {
            parent
                .extensions()
                .get::<SpanContexts>()
                .and_then(|contexts| contexts.get(&self.0))
                .map(|data| data.id)
        });

        let mut fields = Fields::default();

        attrs.record(&mut fields);
        let data = SpanData {
            id: self.0.0.next_span.fetch_add(1, Ordering::Relaxed),
            parent,
            started: Instant::now(),
            fields: fields.0,
        };

        let mut extensions = span.extensions_mut();

        if extensions.get_mut::<SpanContexts>().is_none() {
            extensions.insert(SpanContexts::default());
        }

        extensions
            .get_mut::<SpanContexts>()
            .expect("inserted span contexts")
            .0
            .push((Arc::downgrade(&self.0.0), data));
        drop(extensions);
        self.0.record_context(
            capture(span, &self.0),
            attrs.metadata().level().as_str(),
            attrs.metadata().target(),
            "span_open",
            Vec::new(),
            BTreeMap::new(),
            None,
        );
    }

    fn on_record(&self, id: &Id, values: &SpanRecord<'_>, cx: Context<'_, S>) {
        if let Some(span) = cx.span(id) {
            let mut fields = Fields::default();

            values.record(&mut fields);

            if let Some(data) = span
                .extensions_mut()
                .get_mut::<SpanContexts>()
                .and_then(|contexts| contexts.get_mut(&self.0))
            {
                data.fields.extend(fields.0);
            }
        }
    }

    fn on_event(&self, event: &Event<'_>, cx: Context<'_, S>) {
        let mut fields = Fields::default();

        event.record(&mut fields);
        let mut context = cx
            .event_span(event)
            .map(|span| capture(span, &self.0))
            .unwrap_or_default();

        context.source =
            event
                .metadata()
                .file()
                .zip(event.metadata().line())
                .map(|(file, line)| Source {
                    file: file.into(),
                    line,
                });
        self.0.record_context(
            context,
            event.metadata().level().as_str(),
            event.metadata().target(),
            "event",
            Vec::new(),
            fields.0,
            None,
        );
    }

    fn on_close(&self, id: Id, cx: Context<'_, S>) {
        if let Some(span) = cx.span(&id) {
            let duration = span
                .extensions()
                .get::<SpanContexts>()
                .and_then(|contexts| contexts.get(&self.0))
                .map(|data| data.started.elapsed().as_micros().min(u64::MAX as u128) as u64);

            let metadata = span.metadata();
            let context = capture(span, &self.0);

            self.0.record_context(
                context,
                metadata.level().as_str(),
                metadata.target(),
                "span_close",
                Vec::new(),
                BTreeMap::new(),
                duration,
            );
        }
    }
}

/// Controls whether the runner installs a subscriber or uses the embedding host.
#[derive(Clone, Copy, Debug, Default)]
pub enum SubscriberMode {
    /// Install Rake's subscriber, reporting conflicts as startup errors.
    #[default]
    Automatic,
    /// The host installs [`Collector::layer`] on its own subscriber.
    Host,
}

/// Runner diagnostic configuration. No development control endpoint is installed.
#[derive(Clone, Default)]
#[must_use]
pub struct DiagnosticsConfig {
    collector: Collector,
    mode: SubscriberMode,
}

impl DiagnosticsConfig {
    /// Uses bounded collection and automatic subscriber installation.
    pub fn new() -> Self {
        Self::default()
    }

    /// Selects a collector shared with a host or inspector.
    pub fn collector(mut self, collector: Collector) -> Self {
        self.collector = collector;

        self
    }

    /// Returns a shared collector handle for composing application services.
    #[must_use]
    pub fn shared_collector(&self) -> Collector {
        self.collector.clone()
    }

    /// Selects subscriber ownership.
    pub fn subscriber(mut self, mode: SubscriberMode) -> Self {
        self.mode = mode;

        self
    }

    /// Installs diagnostics once. Automatic installation returns an error on conflict.
    ///
    /// # Errors
    /// Returns [`DiagnosticsError::SubscriberAlreadyInstalled`] if another subscriber
    /// is installed, or [`DiagnosticsError::OutputWorker`] if the output worker cannot start.
    pub fn install(&self) -> Result<(), DiagnosticsError> {
        if matches!(self.mode, SubscriberMode::Host) {
            return Ok(());
        }

        tracing::subscriber::set_global_default(
            Registry::default()
                .with(self.collector.layer())
                .with(OutputLayer::new(self.collector.clone())?)
                .with(tracing_subscriber::filter::LevelFilter::INFO),
        )
        .map_err(|_| DiagnosticsError::SubscriberAlreadyInstalled)
    }
}

/// Diagnostic bootstrap failure.
#[derive(Debug, thiserror::Error)]
pub enum DiagnosticsError {
    /// A host subscriber already exists; select host mode and attach the collector layer.
    #[error(
        "a tracing subscriber is already installed; use SubscriberMode::Host and attach Collector::layer"
    )]
    SubscriberAlreadyInstalled,
    /// The native nonblocking output worker could not be created.
    #[error("could not start the diagnostic output worker: {0}")]
    OutputWorker(#[source] std::io::Error),
}

struct OutputLayer {
    #[cfg(not(target_family = "wasm"))]
    sender: std::sync::mpsc::SyncSender<String>,
    #[cfg(not(target_family = "wasm"))]
    collector: Collector,
}

impl OutputLayer {
    fn new(collector: Collector) -> Result<Self, DiagnosticsError> {
        #[cfg(not(target_family = "wasm"))]
        {
            let (sender, receiver) = std::sync::mpsc::sync_channel::<String>(1024);

            std::thread::Builder::new()
                .name("rake-diagnostics".into())
                .spawn(move || {
                    use std::io::Write;

                    for text in receiver {
                        let _ = writeln!(std::io::stderr().lock(), "{text}");
                    }
                })
                .map_err(DiagnosticsError::OutputWorker)?;

            Ok(Self { sender, collector })
        }

        #[cfg(target_family = "wasm")]
        {
            let _ = collector;

            Ok(Self {})
        }
    }
}

impl<S: Subscriber> Layer<S> for OutputLayer {
    fn on_event(&self, event: &Event<'_>, _cx: Context<'_, S>) {
        let mut fields = Fields::default();

        event.record(&mut fields);
        let text = format!(
            "{} {} {:?}",
            event.metadata().level(),
            event.metadata().target(),
            fields.0
        );

        #[cfg(not(target_family = "wasm"))]
        if self.sender.try_send(text).is_err() {
            self.collector
                .0
                .output_dropped
                .fetch_add(1, Ordering::Relaxed);
        }

        #[cfg(target_family = "wasm")]
        web_sys::console::log_1(&text.into());
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
mod tests;
