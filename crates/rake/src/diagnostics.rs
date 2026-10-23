//! Bounded structured diagnostics, independent of authoritative application state.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{Layer, Registry};
use web_time::Instant;

/// One owned diagnostic record. Values are retained as typed JSON where available.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    /// Monotonic sequence within this collector; not a router revision.
    pub sequence: u64,
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
        }))
    }

    /// Copies retained records without draining the collector.
    pub fn snapshot(&self) -> Snapshot {
        let buffer = self
            .0
            .buffer
            .lock()
            .unwrap_or_else(|error| error.into_inner());

        Snapshot {
            records: buffer.records.iter().cloned().collect(),
            dropped: self.0.dropped.load(Ordering::Relaxed),
            output_dropped: self.0.output_dropped.load(Ordering::Relaxed),
        }
    }

    /// Supplies a layer for a host-owned tracing subscriber.
    pub fn layer(&self) -> CollectorLayer {
        CollectorLayer(self.clone())
    }

    pub(crate) fn record(
        &self,
        level: &str,
        target: &str,
        kind: &str,
        spans: Vec<String>,
        fields: BTreeMap<String, Value>,
    ) {
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
            elapsed_us: self.0.started.elapsed().as_micros().min(u64::MAX as u128) as u64,
            level: level.into(),
            target: target.into(),
            kind: kind.into(),
            spans,
            fields,
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

impl<S: Subscriber + for<'lookup> LookupSpan<'lookup>> Layer<S> for CollectorLayer {
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, cx: Context<'_, S>) {
        let mut fields = Fields::default();

        attrs.record(&mut fields);
        let spans = cx.span_scope(id).map_or_else(Vec::new, |scope| {
            scope.from_root().map(|span| span.name().into()).collect()
        });

        self.0.record(
            attrs.metadata().level().as_str(),
            attrs.metadata().target(),
            "span_open",
            spans,
            fields.0,
        );
    }

    fn on_event(&self, event: &Event<'_>, cx: Context<'_, S>) {
        let mut fields = Fields::default();

        event.record(&mut fields);
        let spans = cx.event_scope(event).map_or_else(Vec::new, |scope| {
            scope.from_root().map(|span| span.name().into()).collect()
        });

        self.0.record(
            event.metadata().level().as_str(),
            event.metadata().target(),
            "event",
            spans,
            fields.0,
        );
    }

    fn on_close(&self, id: Id, cx: Context<'_, S>) {
        if let Some(span) = cx.span(&id) {
            self.0.record(
                span.metadata().level().as_str(),
                span.metadata().target(),
                "span_close",
                span.scope()
                    .from_root()
                    .map(|span| span.name().into())
                    .collect(),
                BTreeMap::new(),
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
pub struct DiagnosticsConfig {
    pub(crate) collector: Collector,
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

    /// Selects subscriber ownership.
    pub fn subscriber(mut self, mode: SubscriberMode) -> Self {
        self.mode = mode;

        self
    }

    /// Installs diagnostics once. Automatic installation returns an error on conflict.
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
