//! Shared crash configuration and native panic recording, without a UI runtime.
//!
//! Configuration, record and sink types are available on native and Web. Native
//! filesystem operations (`CrashConfig::open`, `CrashReporter`) are native-only.
//! Web delivery is the follow-up from issue #16; the Web App builder is a no-op.
//!
//! The hook records locally; delivery policy belongs to the application. Arbitrary
//! panic payloads and diagnostic content are omitted for privacy. Successful delivery
//! deletes a record; failed or interrupted delivery can be retried by stable ID.
//!
//! ```no_run
//! use rake::crash::{CrashConfig, CrashLimits, CrashRecord};
//! use rake::diagnostics::Collector;
//! # fn main() -> std::io::Result<()> {
//! # #[cfg(not(target_family = "wasm"))] {
//! let reporter = CrashConfig::new(env!("CARGO_PKG_VERSION"))
//!     .directory("/private/app-owned/crashes")
//!     .limits(CrashLimits { records: 4, ..CrashLimits::default() })
//!     .open("com.example.editor")?;
//! reporter.install(Collector::new(128))?;
//! reporter.deliver(&|record: &CrashRecord| {
//!     eprintln!("Previous panic: {}", record.id);
//!     Ok(())
//! })?;
//! # }
//! # Ok(()) }
//! ```

use std::io;
use std::path::PathBuf;

use crate::diagnostics::CrashDiagnostic;
use serde::{Deserialize, Serialize};

#[cfg(not(target_family = "wasm"))]
mod native;

#[cfg(not(target_family = "wasm"))]
pub use native::CrashReporter;

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

/// Shared crash configuration; the Web builder ignores it until issue #16 Web delivery.
#[derive(Clone, Debug)]
#[must_use]
pub struct CrashConfig {
    directory: Option<PathBuf>,
    #[cfg_attr(
        target_family = "wasm",
        expect(dead_code, reason = "Web delivery is the follow-up from issue #16")
    )]
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

    /// Replaces retention budgets; invalid byte/count budgets fail when opening a native spool.
    pub fn limits(mut self, limits: CrashLimits) -> Self {
        self.limits = limits;

        self
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
