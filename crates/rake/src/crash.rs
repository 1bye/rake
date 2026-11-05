//! Opt-in native panic records and application-owned delivery, without a UI runtime.

//! The hook records locally; delivery policy belongs to the application. Arbitrary
//! panic payloads and diagnostic content are omitted for privacy. Successful delivery
//! deletes a record; failed or interrupted delivery can be retried by stable ID.
//!
//! ```no_run
//! use rake::crash::{CrashConfig, CrashLimits, CrashRecord};
//! use rake::diagnostics::Collector;
//! # fn main() -> std::io::Result<()> {
//! let reporter = CrashConfig::new(env!("CARGO_PKG_VERSION"))
//!     .directory("/private/app-owned/crashes")
//!     .limits(CrashLimits { records: 4, ..CrashLimits::default() })
//!     .open("com.example.editor")?;
//! reporter.install(Collector::new(128))?;
//! reporter.deliver(&|record: &CrashRecord| {
//!     eprintln!("Previous panic: {}", record.id);
//!     Ok(())
//! })?;
//! # Ok(()) }
//! ```

use std::backtrace::Backtrace;
use std::fmt::{self, Write as _};
use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::diagnostics::{Collector, CrashDiagnostic};
use crate::storage::file::{check_directory, check_file, private_options};
use serde::{Deserialize, Serialize};

static INSTALLED: AtomicBool = AtomicBool::new(false);
static NEXT_RECORD: AtomicU64 = AtomicU64::new(0);
const PRIVATE_MESSAGE: &str = "Rust panic (payload omitted for privacy)";
const MAX_BACKTRACE_BYTES: usize = 32 * 1024;

/// Resource budgets for native records; validated when opening the directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CrashLimits {
    /// Maximum encoded JSON bytes per record, from 1024 to 1 MiB.
    pub record_bytes: usize,
    /// Maximum retained records, from 1 to 128; oldest unlocked records are removed.
    pub records: usize,
    /// Maximum structural collector entries, clamped to 128.
    pub diagnostics: usize,
}

impl Default for CrashLimits {
    fn default() -> Self {
        Self {
            record_bytes: 64 * 1024,
            records: 8,
            diagnostics: 32,
        }
    }
}

/// Native crash recording configuration. Creating it has no side effects.
#[derive(Clone, Debug)]
#[must_use]
pub struct CrashConfig {
    directory: Option<PathBuf>,
    version: String,
    limits: CrashLimits,
}

impl CrashConfig {
    /// Supplies the application's version (usually `env!("CARGO_PKG_VERSION")`).
    pub fn new(version: impl Into<String>) -> Self {
        Self {
            directory: None,
            version: version.into(),
            limits: CrashLimits::default(),
        }
    }

    /// Overrides the crash directory; it must be application-owned and trusted.
    pub fn directory(mut self, directory: impl Into<PathBuf>) -> Self {
        self.directory = Some(directory.into());

        self
    }

    /// Replaces retention budgets; invalid byte/count budgets fail in [`Self::open`].
    pub fn limits(mut self, limits: CrashLimits) -> Self {
        self.limits = limits;

        self
    }

    /// Opens/creates a private directory without installing a hook or delivering records.
    ///
    /// Defaults to `XDG_DATA_HOME/ID/crashes` (or `~/.local/share`), macOS
    /// `~/Library/Application Support/ID/crashes`, or Windows `APPDATA/ID/crashes`.
    /// Environment values resolve the directory only and are never recorded.
    ///
    /// # Errors
    /// Returns an I/O error for invalid limits/IDs, missing data roots, symlinks,
    /// non-private Unix permissions, or filesystem failures. IDs accept 1–128 ASCII
    /// letters, digits, dots, underscores and hyphens, excluding `.` and `..`.
    pub fn open(self, application_id: &str) -> io::Result<CrashReporter> {
        if application_id.is_empty()
            || application_id.len() > 128
            || matches!(application_id, "." | "..")
            || !application_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
            || !(1024..=1024 * 1024).contains(&self.limits.record_bytes)
            || !(1..=128).contains(&self.limits.records)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid crash ID or limits",
            ));
        }

        let directory = match self.directory {
            Some(directory) => directory,
            None => data_root()?.join(application_id).join("crashes"),
        };

        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }

        builder.create(&directory)?;
        check_directory(&directory)?;
        let reporter = CrashReporter {
            directory: fs::canonicalize(directory)?,
            version: prefix(&self.version, 128).into(),
            version_truncated: self.version.len() > 128,
            limits: self.limits,
        };

        reporter.prune()?;
        Ok(reporter)
    }
}

/// One versioned, owned native crash record containing no application content.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CrashRecord {
    /// JSON schema version; currently 1.
    pub schema_version: u32,
    /// Stable opaque identity; sinks can use it to deduplicate retries.
    pub id: String,
    /// Time since Unix epoch in milliseconds, or zero if unavailable.
    pub timestamp_ms: u64,
    /// Privacy marker; arbitrary panic payloads are never retained.
    pub message: String,
    /// Rust source basename and line/column, without source directory paths.
    pub location: Option<String>,
    /// Bounded standard-library backtrace with source-path lines omitted.
    pub backtrace: String,
    /// Application-supplied build version, limited to 128 UTF-8 bytes.
    pub app_version: String,
    /// Compile-time operating-system name; no hostname or environment values.
    pub os: String,
    /// Structural collector tail, or `None` when the collector lock was unavailable.
    pub diagnostics: Option<Vec<CrashDiagnostic>>,
    /// Whether any metadata was shortened or removed to meet byte budgets.
    pub truncated: bool,
}

/// Application policy for a waiting native crash record; Rake never uploads it.
///
/// Called synchronously on startup, outside the panic hook. For UI prompts, first
/// persist/enqueue the record in application-owned storage and return success only
/// when that handoff is durable. Use the record ID for idempotent side effects.
pub trait CrashSink: 'static {
    /// Accepts a record. Success deletes it; failure retains it for a later launch.
    ///
    /// # Errors
    /// Return an I/O error when the handoff failed or consent should be retried.
    fn report(&self, record: &CrashRecord) -> io::Result<()>;
}

impl<F: Fn(&CrashRecord) -> io::Result<()> + 'static> CrashSink for F {
    fn report(&self, record: &CrashRecord) -> io::Result<()> {
        self(record)
    }
}

/// Shared native directory/configuration handle; cloning does not install another hook.
#[derive(Clone, Debug)]
pub struct CrashReporter {
    directory: PathBuf,
    version: String,
    version_truncated: bool,
    limits: CrashLimits,
}

impl CrashReporter {
    /// Installs one process-wide hook and chains to the previously installed hook.
    ///
    /// The hook never invokes a sink, tracing, panic payload formatting, or blocking
    /// application locks. I/O failures are ignored so the original panic can proceed.
    /// Keep the directory trusted for the process lifetime. Installation persists
    /// until process exit; call during startup, never from a panicking thread.
    ///
    /// # Errors
    /// Returns `AlreadyExists` if Rake already installed a crash hook in this process,
    /// or `InvalidInput` if called from a panicking thread.
    pub fn install(&self, collector: Collector) -> io::Result<()> {
        if std::thread::panicking() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cannot install during panic",
            ));
        }

        if INSTALLED.swap(true, Ordering::AcqRel) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "crash hook already installed",
            ));
        }

        let reporter = self.clone();
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // No catch_unwind here: Rust aborts before unwinding a panic in a hook.
            // Only owned primitive metadata is formatted; no application callbacks run.
            let location = info.location().map(|location| {
                format!(
                    "{}:{}:{}",
                    basename(location.file()),
                    location.line(),
                    location.column()
                )
            });
            let record = reporter.record(
                location,
                collector.try_crash_tail(reporter.limits.diagnostics),
            );
            let _ = reporter.write(record);
            previous(info);
        }));

        Ok(())
    }

    /// Delivers waiting records in filename order; a busy delivery process returns zero.
    ///
    /// An advisory process lock and atomic claim rename prevent concurrent delivery.
    /// Interrupted claims are retried on the next call. Success removes the record;
    /// failure (including sink unwinding) restores it. Corrupt, oversized and unknown
    /// schema files are removed without calling the sink. No filesystem locks block.
    /// A process death after a sink side effect but before deletion can cause a retry;
    /// exactly-once external effects require an idempotent sink keyed by record ID.
    ///
    /// # Errors
    /// Returns the first sink or filesystem error, leaving undelivered records intact.
    pub fn deliver(&self, sink: &dyn CrashSink) -> io::Result<usize> {
        check_directory(&self.directory)?;
        let lock = private_options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.directory.join("delivery.lock"))?;
        check_file(&lock)?;

        match lock.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Ok(0),
            Err(std::fs::TryLockError::Error(error)) => return Err(error),
        }

        let mut delivered = 0;

        for path in self.records()? {
            let file = match private_options().read(true).write(true).open(&path) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };

            check_file(&file)?;

            if file.try_lock().is_err() {
                continue;
            }

            let claimed = path.with_extension("claimed");

            if path != claimed {
                match fs::rename(&path, &claimed) {
                    Ok(()) => {}
                    // Retention may have removed it before we acquired its lock.
                    Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error),
                }
            }

            let claim = Claim {
                claimed,
                pending: path.with_extension("json"),
            };

            let mut bytes = Vec::new();
            // Borrow the locked file so its lock lives through the sink callback.
            (&file)
                .take((self.limits.record_bytes + 1) as u64)
                .read_to_end(&mut bytes)?;
            let record = if bytes.len() <= self.limits.record_bytes {
                serde_json::from_slice::<CrashRecord>(&bytes)
                    .ok()
                    .filter(|record| {
                        record.schema_version == 1
                            && !record.id.is_empty()
                            && record.id.len() <= 96
                            && record
                                .id
                                .bytes()
                                .all(|byte| byte.is_ascii_digit() || byte == b'-')
                    })
            } else {
                None
            };

            if let Some(record) = record {
                sink.report(&record)?;
                delivered += 1;
            }

            fs::remove_file(&claim.claimed)?;
        }

        Ok(delivered)
    }

    fn record(
        &self,
        location: Option<String>,
        diagnostics: Option<Vec<CrashDiagnostic>>,
    ) -> CrashRecord {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let mut trace = TraceText {
            text: String::new(),
            truncated: false,
        };

        let _ = write!(trace, "{}", Backtrace::force_capture());
        let location_truncated = location.as_ref().is_some_and(|text| text.len() > 256);
        CrashRecord {
            schema_version: 1,
            id: format!(
                "{:032}-{:010}-{:020}",
                timestamp.as_nanos(),
                std::process::id(),
                NEXT_RECORD.fetch_add(1, Ordering::Relaxed)
            ),
            timestamp_ms: timestamp.as_millis().min(u128::from(u64::MAX)) as u64,
            message: PRIVATE_MESSAGE.into(),
            location: location.map(|text| prefix(&text, 256).into()),
            backtrace: trace
                .text
                .lines()
                .filter(|line| !line.trim_start().starts_with("at "))
                .collect::<Vec<_>>()
                .join("\n"),
            app_version: self.version.clone(),
            os: std::env::consts::OS.into(),
            diagnostics,
            truncated: trace.truncated || self.version_truncated || location_truncated,
        }
    }

    fn write(&self, mut record: CrashRecord) -> io::Result<()> {
        check_directory(&self.directory)?;
        let bytes = loop {
            let bytes = serde_json::to_vec(&record).map_err(io::Error::other)?;

            if bytes.len() <= self.limits.record_bytes {
                break bytes;
            }

            record.truncated = true;

            if let Some(tail) = record.diagnostics.as_mut()
                && !tail.is_empty()
            {
                tail.remove(0);
            } else if !record.backtrace.is_empty() {
                record.backtrace.truncate(
                    record
                        .backtrace
                        .floor_char_boundary(record.backtrace.len() / 2),
                );
            } else if let Some(location) = record.location.as_mut()
                && !location.is_empty()
            {
                location.truncate(location.floor_char_boundary(location.len() / 2));
            } else if !record.app_version.is_empty() {
                record.app_version.truncate(
                    record
                        .app_version
                        .floor_char_boundary(record.app_version.len() / 2),
                );
            } else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "crash metadata exceeds budget",
                ));
            }
        };

        let temporary = Temporary(self.directory.join(format!(".{}.tmp", record.id)));
        let mut file = private_options()
            .write(true)
            .create_new(true)
            .open(&temporary.0)?;
        check_file(&file)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(
            &temporary.0,
            self.directory.join(format!("{}.json", record.id)),
        )?;
        let marker = private_options()
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.directory.join("retention.pending"))?;
        check_file(&marker)?;
        drop(marker);

        self.prune()
    }

    fn records(&self) -> io::Result<Vec<PathBuf>> {
        let mut paths = Vec::new();

        for entry in fs::read_dir(&self.directory)? {
            let path = entry?.path();

            if path
                .extension()
                .is_some_and(|extension| extension == "json" || extension == "claimed")
            {
                paths.push(path);
            }
        }

        paths.sort();

        Ok(paths)
    }

    fn prune(&self) -> io::Result<()> {
        // A writer marks pending AFTER commit, BEFORE trying the lock. The owner
        // clears it BEFORE scanning and checks again AFTER unlock, so a contending
        // writer cannot leave an unobserved commit when all writers have finished.
        loop {
            check_directory(&self.directory)?;
            let lock = private_options()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(self.directory.join("retention.lock"))?;
            check_file(&lock)?;

            match lock.try_lock() {
                Ok(()) => {}
                Err(std::fs::TryLockError::WouldBlock) => return Ok(()),
                Err(std::fs::TryLockError::Error(error)) => return Err(error),
            }

            let pending = self.directory.join("retention.pending");

            match fs::remove_file(&pending) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }

            let paths = self.records()?;
            let mut excess = paths.len().saturating_sub(self.limits.records);

            for path in paths {
                if excess == 0 {
                    break;
                }

                let Ok(file) = private_options().read(true).write(true).open(&path) else {
                    continue;
                };

                if check_file(&file).is_err() || file.try_lock().is_err() {
                    continue;
                }

                match fs::remove_file(path) {
                    Ok(()) => excess -= 1,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }

            drop(lock);

            if !pending.try_exists()? {
                return Ok(());
            }
        }
    }
}

struct Claim {
    claimed: PathBuf,
    pending: PathBuf,
}

impl Drop for Claim {
    fn drop(&mut self) {
        let _ = fs::rename(&self.claimed, &self.pending);
    }
}

struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

struct TraceText {
    text: String,
    truncated: bool,
}

impl fmt::Write for TraceText {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let available = MAX_BACKTRACE_BYTES.saturating_sub(self.text.len());
        let part = prefix(text, available);
        self.text.push_str(part);
        self.truncated |= part.len() < text.len();

        Ok(())
    }
}

fn prefix(text: &str, bytes: usize) -> &str {
    &text[..text.floor_char_boundary(bytes.min(text.len()))]
}

fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or("unknown")
}

fn data_root() -> io::Result<PathBuf> {
    #[cfg(target_os = "windows")]
    let root = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let root = std::env::var_os("HOME")
        .map(|root| PathBuf::from(root).join("Library/Application Support"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let root = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|root| root.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|root| PathBuf::from(root).join(".local/share")));
    root.filter(|root| root.is_absolute())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no absolute app-data root"))
}

#[cfg(test)]
mod tests;
