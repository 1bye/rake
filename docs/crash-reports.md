# Native crash reports

Native panic records are **opt-in**. Rake writes locally and never uploads a report.
The configuration, limits, record and sink types in `rake::crash` are available on
native and Web, including with `--no-default-features`. Native filesystem operations
(`CrashConfig::open`, `CrashReporter`, `install`, `deliver`) remain native-only and
need no GPUI or display. `App::crash_reports` is a no-op on Web: it stores neither
configuration nor sink and performs no hook installation, file I/O or delivery.
Shared native/Web App builders can call it without target-specific `cfg` guards.
Web sink delivery is the follow-up from issue #16; the existing Web console hook
is unchanged.
With GPUI, configure the application before calling `run`:

```rust,no_run
use rake::crash::{CrashConfig, CrashRecord};

# #[cfg(feature = "gpui")] {
let app = rake::App::new("com.example.editor").crash_reports(
    CrashConfig::new(env!("CARGO_PKG_VERSION")),
    |record: &CrashRecord| {
        // Hand off to application-owned storage before acknowledging success.
        eprintln!("Previous panic: {} ({})", record.id, record.app_version);
        Ok(())
    },
);
// Add root/root_with/routes/background, then app.run().
# }
```

`App` opens the directory, installs the hook using its diagnostics collector and
attempts waiting delivery before native GPUI initialization. Setup errors fail
startup; a sink error leaves the record waiting and startup continues. No UI exists
yet: a graphical application should use a durable inbox sink, then prompt after
mounting. The hook lasts until process exit; installing a second Rake hook returns
`AlreadyExists`. Install before starting worker threads. Hosts that replace the
panic hook later must chain to the installed hook if they want recording to continue.

A headless host controls these steps directly:

```rust,no_run
use rake::crash::{CrashConfig, CrashLimits, CrashRecord};
use rake::diagnostics::Collector;
# fn main() -> std::io::Result<()> {
let reporter = CrashConfig::new(env!("CARGO_PKG_VERSION"))
    .directory("/private/app-owned/crashes")
    .limits(CrashLimits { records: 4, record_bytes: 16 * 1024, diagnostics: 16 })
    .open("com.example.editor")?;
reporter.install(Collector::new(128))?;
reporter.deliver(&|record: &CrashRecord| {
    eprintln!("Previous panic: {}", record.id);
    Ok(())
})?;
# Ok(()) }
```

## File-writing sink

This sink appends JSON lines to an application-selected file and syncs them before
acknowledging. The parent directory must be trusted and private; do not choose a
user-document directory or derive the path from record contents. On Unix the file
is created with mode `0600`, and existing unsafe files/symlinks are rejected.
`FileBackend::open` can prepare/check a private directory before constructing the
sink. The mutex runs during normal delivery, never in the panic hook.

```rust,no_run
use rake::crash::{CrashRecord, CrashSink};
use std::{fs::File, io::{self, Write}, sync::Mutex};

struct FileSink(Mutex<File>);
impl FileSink {
    fn open(path: &std::path::Path) -> io::Result<Self> {
        if let Ok(metadata) = std::fs::symlink_metadata(path) {
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(io::Error::other("report destination must be a regular file"));
            }
        }
        let mut options = std::fs::OpenOptions::new();
        options.append(true).create(true);
        #[cfg(unix)] {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            options.mode(0o600);
            // Parent directory is trusted; prevent pre-existing symlinks above.
            if let Ok(metadata) = std::fs::metadata(path) {
                if metadata.permissions().mode() & 0o077 != 0 {
                    return Err(io::Error::other("report destination must be private"));
                }
            }
        }
        Ok(Self(Mutex::new(options.open(path)?)))
    }
}
impl CrashSink for FileSink {
    fn report(&self, record: &CrashRecord) -> io::Result<()> {
        let mut bytes = serde_json::to_vec(record)?;
        bytes.push(b'\n');
        let mut file = self.0.lock().map_err(|_| io::Error::other("file sink poisoned"))?;
        file.write_all(&bytes)?;
        file.sync_all()
    }
}
# fn main() -> io::Result<()> {
let sink = FileSink::open(std::path::Path::new("/private/app-owned/accepted.jsonl"))?;
# let _ = sink;
# Ok(()) }
```

The log can include repeated IDs if the process dies after writing but before Rake
acknowledges delivery. Deduplicate by `record.id` when consuming the log. Successful
handoff transfers retention/privacy responsibility to the application; Rake's
limits apply only to its crash spool, not the sink's archive.

## Prompt-the-user sink

For a command-line host, a synchronous consent prompt can decide whether to archive,
discard or retry. This example does no uploading and prints only structural metadata:

```rust,no_run
use rake::crash::{CrashRecord, CrashSink};
use std::io::{self, Write};

struct PromptSink;
impl CrashSink for PromptSink {
    fn report(&self, record: &CrashRecord) -> io::Result<()> {
        eprint!("A previous run of version {} panicked. Discard report {}? [y/N] ",
            record.app_version, record.id);
        io::stderr().flush()?;
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        if answer.trim().eq_ignore_ascii_case("y") {
            Ok(()) // Explicit discard acknowledges and removes the spool record.
        } else {
            Err(io::Error::other("defer until next launch"))
        }
    }
}
```

For a GPUI prompt, use the file sink (or an idempotent per-ID inbox) at startup and
load that inbox after the root mounts. Present the application's own dialog there;
its consent decision controls archive/delete/upload policy. Returning `Ok(())` for
an in-memory queue alone can lose reports if the app dies before the prompt.

## Format, limits and privacy

Defaults: 8 records, each at most 64 KiB of encoded JSON, and 32 structural diagnostic
entries (hard cap 128). `CrashLimits` accepts 1–128 records and 1 KiB–1 MiB per record.
Oldest filenames, which begin with capture time and include PID plus a process-local
counter, are removed first. Count trimming uses a nonblocking retention lock and a pending-work marker so
concurrent writers hand trimming to the active owner. In-flight commits can temporarily
exceed the count, which is enforced when writers settle (or on the next open after
an interrupted writer). Records actively locked by delivery are skipped. A currently delivered record is never deleted by retention. Wall-clock
changes can affect chronological ordering. Each record has:

- `schema_version: 1`, stable `id`, Unix `timestamp_ms`, app version and OS name;
- `message`, `Rust panic (payload omitted for privacy)` by default; opt-in string
  payloads are capped at 1 KiB of UTF-8 and may be shortened further by the JSON budget;
- `location`, a Rust source basename with line/column;
- a standard-library backtrace, captured even without `RUST_BACKTRACE`, with source
  path lines removed and formatting capped at 32 KiB;
- optional diagnostic entries: sequence, elapsed microseconds, normalized level and
  kind only; `null` means the collector was contended or poisoned;
- `truncated`, indicating budget truncation.

The app version is developer metadata (at most 128 UTF-8 bytes); supply a version,
not user data. Diagnostic fields, targets, span names, HTTP data, window titles,
editable text, user paths and environment values are excluded. Arbitrary panic
messages may contain those values, so they are omitted by default. To explicitly
accept that privacy risk, configure `CrashConfig::include_panic_message(true)`.
Only `&'static str` and `String` panic payloads are read; other types keep the marker
and are never formatted. The string is capped at 1 KiB at a UTF-8 boundary and marks
`truncated` when shortened. Opting in can persist user content or secrets embedded
in panic strings: the app must own consent, disclosure and retention policy. This
option does not enable diagnostic content capture or any uploading.

The previous panic hook still receives the original payload and prints its usual
output; these privacy rules cover persisted records, not the host's stderr policy.
Source locations and symbol names are developer/build metadata; use compiler path
remapping if other development tools retain full source paths.

If JSON escaping or metadata exceeds the encoded byte budget, oldest diagnostics
are removed first, then opted-in messages, then the backtrace are shortened at UTF-8
boundaries, followed
by location and version if JSON escaping still exceeds the budget. Version/location
metadata also has the independent UTF-8 caps above. Collector capture uses `try_lock` and
never waits on a lock that the panicking thread might already hold. No sink, tracing
subscriber, arbitrary payload formatter or application callback runs in the hook.
Filesystem failures are best-effort and leave the previous hook running. Rust cannot
recover from a second panic inside a panic hook: the implementation avoids panicking
operations rather than attempting `catch_unwind` there. A panicking previous hook,
allocator failure or process abort remains outside the guarantee.

Default locations:

| Platform | Crash directory |
| --- | --- |
| Linux | `$XDG_DATA_HOME/APP_ID/crashes`, falling back to `~/.local/share/APP_ID/crashes` |
| macOS | `~/Library/Application Support/APP_ID/crashes` |
| Windows | `%APPDATA%/APP_ID/crashes` |

An explicit directory is accepted via `CrashConfig::directory`. IDs use the same
safe namespace alphabet as preferences. The directory must be application-owned
and trusted. The implementation reuses native storage's private-file/directory
checks: Unix directories `0700`, files `0600`, final symlinks refused, regular files
only, atomic temp-file replacement after sync. Ancestor directories must be trusted,
as with `FileBackend`. No directory fsync/power-loss durability is promised.
Windows inherits ACLs from the application-owned directory; Unix mode checks do not
establish Windows ACL privacy.

## Delivery and recovery

Delivery obtains a nonblocking process-wide advisory file lock for the directory,
then atomically renames `ID.json` to `ID.claimed` before invoking the sink. The
record file stays locked through the callback so retention cannot remove it. A busy
consumer returns zero. Success deletes the claimed file; an error or sink unwind
restores the pending filename. An interrupted claim is recovered under the delivery
lock on a later launch. A failed sink stops that delivery pass. Invalid/truncated JSON,
oversized records and invalid older schema versions are discarded without invoking
the sink. Valid JSON with `schema_version > 1` is preserved byte-for-byte, restored
to `.json` and not counted or sent to the sink, even if its layout is unfamiliar.
This protects records across app downgrades. Newer-schema records remain subject
to ordinary oldest-first retention.

Under normal completion, successive launches deliver a record once, and concurrent
processes cannot both deliver it. **Exactly-once external side effects cannot be
promised across process death:** a sink may commit and the process may die before
Rake deletes the claim. Make the sink idempotent by ID where this matters. Errors
from deleting an acknowledged record can cause the same retry. A directory busy at
startup is retried on a later launch, or by calling `CrashReporter::deliver` again.

The Web `console_error_panic_hook` behavior is unchanged. Delivering Web records to
the same sink is a follow-up. Segfaults, aborts, signal handlers, minidumps, vendor
integrations and OpenTelemetry export are outside this implementation. Validation
was performed on Linux (devbox-ts, Ubuntu 24.04); macOS and Windows need verification.
