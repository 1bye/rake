//! Native panic hook, private spool and claimed delivery.

use std::backtrace::Backtrace;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{CrashConfig, CrashLimits, CrashRecord, CrashSink};
use crate::diagnostics::{Collector, CrashDiagnostic};
use crate::storage::file::{check_directory, check_file, private_options};

static INSTALLED: AtomicBool = AtomicBool::new(false);
static NEXT_RECORD: AtomicU64 = AtomicU64::new(0);
use super::record::*;

impl CrashConfig {
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
        self.validate(application_id)?;

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
            include_panic_message: self.include_panic_message,
        };

        reporter.prune()?;
        Ok(reporter)
    }
}

/// Shared native directory/configuration handle; cloning does not install another hook.
#[derive(Clone, Debug)]
pub struct CrashReporter {
    directory: PathBuf,
    version: String,
    version_truncated: bool,
    limits: CrashLimits,
    include_panic_message: bool,
}

impl CrashReporter {
    /// Installs one process-wide hook and chains to the previously installed hook.
    ///
    /// Records every panic after installation, including panics caught by
    /// `catch_unwind` and worker-thread panics while the application keeps running.
    /// A record establishes that a panic occurred, not that the app exited.
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
                    source_path(location.file()),
                    location.line(),
                    location.column()
                )
            });
            let mut record = reporter.record(
                location,
                collector.try_crash_tail(reporter.limits.diagnostics),
            );
            let (message, truncated) =
                panic_message(info.payload(), reporter.include_panic_message);
            record.message = message;
            record.truncated |= truncated;
            let _ = reporter.write(record);
            previous(info);
        }));

        Ok(())
    }

    /// Delivers waiting records in filename order; a busy delivery process returns zero.
    ///
    /// An advisory process lock and atomic claim rename prevent concurrent delivery.
    /// Interrupted claims are retried on the next call. Success removes the record;
    /// failure (including sink unwinding) restores it. Newer JSON schema versions are
    /// retained without calling the sink, including layouts this version cannot read.
    /// Corrupt, oversized and invalid older files are removed. No filesystem locks block.
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

        let _delivery_lock = FileLock(&lock);
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

            let _record_lock = FileLock(&file);
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
            if bytes.len() <= self.limits.record_bytes
                && serde_json::from_slice::<RecordVersion>(&bytes)
                    .is_ok_and(|record| record.schema_version > 1)
            {
                continue;
            }

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
        let bytes = encode(&mut record, self.limits.record_bytes)?;

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
        marker.set_len(1)?;

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
        check_directory(&self.directory)?;
        // Never unlink the shared notification: Windows rejects opens while an
        // existing handle keeps a deleted file pending. Its length is a one-bit flag.
        let pending = private_options()
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.directory.join("retention.pending"))?;
        check_file(&pending)?;

        // Mark AFTER commit, clear BEFORE scanning, check AFTER unlock. A writer
        // either appears in this scan, notifies this owner, or becomes the next owner.
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

            let retention_lock = FileLock(&lock);
            pending.set_len(0)?;

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

                let _record_lock = FileLock(&file);

                match fs::remove_file(path) {
                    Ok(()) => excess -= 1,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }

            drop(retention_lock);
            drop(lock);

            if pending.metadata()?.len() == 0 {
                return Ok(());
            }
        }
    }
}

// Closing alone leaves flock held by a descriptor inherited across fork until exec.
// Explicit unlock also releases those references, including when a sink unwinds.
struct FileLock<'a>(&'a fs::File);

impl Drop for FileLock<'_> {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

#[derive(serde::Deserialize)]
struct RecordVersion {
    schema_version: u64,
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
#[path = "tests.rs"]
mod tests;
