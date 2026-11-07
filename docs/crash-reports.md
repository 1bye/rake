# Crash reports

Native and Web GPUI **App recording is on by default; delivery is opt-in**. Rake writes
bounded local panic records to the app's data directory (native) or localStorage (Web) without being asked, in
both development and packaged runs. It never uploads anything. Panic payloads are
omitted by default, and the normal size/count/privacy limits below apply.

With no crash builder calls, `App::run` opens the default crash spool and
installs the hook with the App's diagnostics collector before GPUI starts.
There is no sink, delivery pass or acknowledgement deletion: records wait for a
later run that configures a sink. Ordinary oldest-first retention still applies.
Default storage/hook setup failures emit a warning and startup continues, including
when a host already installed a Rake hook. Constructing an App alone performs no I/O.

The configuration, limits, record and sink types in `rake::crash` are available on
native and Web, including with `--no-default-features`. Native filesystem operations
(`CrashConfig::open`, `CrashReporter`, `install`, `deliver`) remain native-only.
Headless native hosts still opt into installation and delivery explicitly; no implicit
headless hook is installed. Web App builders use the same defaults, limits, sink
ownership and last-call-wins policy. The Web console panic hook remains enabled,
including when Rake recording is disabled.

To opt into delivery and replace the default directory, limits or privacy options,
configure an explicit sink before `run`:

```rust,no_run
use rake::crash::CrashRecord;

# #[cfg(feature = "gpui")] {
let app = rake::App::new("com.example.editor").crash_reports(
    rake::crash_config!(),
    |record: &CrashRecord| {
        // Hand off to application-owned storage before acknowledging success.
        eprintln!("Previous panic: {} ({})", record.id, record.app_version);
        Ok(())
    },
);
// Add root/root_with/routes/background, then app.run().
# }
```

`rake::crash_config!()` captures `CARGO_PKG_VERSION` in the caller's crate. The
implicit default instead uses the consumer version already embedded in the
build-script `RAKE_UPDATE_POLICY` and passed through `App::distribution_policy`
(automatically by `#[rake::app]`). This works without enabling update services.
When that metadata is absent or invalid, the default records `"unknown"`, never
Rake's own version. There is no new build-script requirement. Manual entry points
can pass existing metadata through `distribution_policy`, or use the macro with an
explicit sink to supply their own Cargo version.

Explicit `App::crash_reports(config, sink)` setup errors still fail startup; sink
errors retain records and startup continues. Native delivery runs before UI initialization; Web delivery is scheduled asynchronously
at startup under a Web Lock. In both cases the sink itself is synchronous:
use a durable inbox, then prompt after mounting. An app that adds a sink later can
deliver older default records by using the same app ID and default directory.

Opt out when another reporter such as Sentry or a host-owned hook owns reporting:

```rust,no_run
# #[cfg(feature = "gpui")] {
let app = rake::App::new("com.example.editor").without_crash_reports();
// Add the application's root, routes or background entry point, then run.
# }
```

Opt-out creates no crash directory and installs no hook; it does not remove a hook
already installed by the host. **Last call wins**: `.crash_reports(...).without_crash_reports()`
disables Rake setup and drops the sink, while `.without_crash_reports().crash_reports(...)`
enables the explicit configuration and delivery.

The hook lasts until process exit (or Web page disposal). **Any hook installed later must chain to Rake's
hook or recording stops**, including apps using default recording without a sink.
Installing a second Rake hook returns `AlreadyExists`; the automatic App path warns
and continues, while the explicit path fails startup. Install host hooks before
launch or chain them deliberately.

The native hook records **every panic in the process after installation**, including
panics caught by `catch_unwind` (such as recovery paths in `rake-updates`) and panics
on background threads while the rest of the application continues running. A crash
record proves that a panic occurred; it does **not** prove the app exited. Sinks and
consent UI should describe it as a previous panic rather than assume an app crash.
Thread names are not recorded: the structural diagnostic tail and source location
remain the context, without adding a new developer-supplied string to the format.

A headless host controls these steps directly:

```rust,no_run
use rake::crash::{CrashLimits, CrashRecord};
use rake::diagnostics::Collector;
# fn main() -> std::io::Result<()> {
let reporter = rake::crash_config!()
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
- `location`, the relative Rust source path with line/column, capped at 256 UTF-8
  bytes; absolute paths keep only `crate/src/...` after the last source root, or
  fall back to the basename (both Unix and Windows path separators are handled);
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
Relative source locations and symbol names are developer/build metadata. Absolute
source locations omit the build-machine prefix. Backtrace `at <path>` lines are
still dropped entirely; only the panic location uses the more useful path rule. Use compiler path
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

Default native locations:

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

## Web storage and browser limits

Web Apps synchronously save panic records to **origin-scoped localStorage**.
Keys are `rake:crashes:APP_ID:RECORD_ID`; values contain a pending (`P`) or claimed
(`C`) marker followed by the same schema-v1 JSON as native. A native
`CrashConfig::directory` override is ignored: the app ID is the browser namespace.
Apps sharing an origin and ID share this spool. Other origin scripts can read
or modify it; storage is unencrypted. Browser policy, eviction, clearing site data
or private-session disposal can remove it.

The same size/count/diagnostic budgets and privacy filters apply on both platforms:
8 records / 64 KiB encoded JSON per record / 32 structural entries by default.
The state marker adds one character. Browser IDs use time, a random per-page
identity generated before hook installation, and a counter. Wasm has no
standard-library backtrace, so that field is empty. JavaScript stacks, page URLs,
user agents and browser/OS fingerprint strings are not persisted; `os` is the
compile-time Rust target OS. Panic strings remain omitted unless explicitly enabled.
Quota/denied-storage failures during a panic are ignored and do not cause another
panic. Explicit setup errors fail startup; default setup errors warn and continue.

Delivery holds the exclusive Web Lock `rake:crashes:APP_ID:spool` across the
synchronous sink callback. A busy lock skips that pass; a later load retries.
A single atomic value replacement claims a record; success deletes it, failure
or a throwing/trapping sink restores pending state, and a claim interrupted by
page disposal retries on a later load. Future schemas are preserved byte-for-byte;
corrupt/oversized records are discarded without invoking the sink. Retention holds
the same lock so it cannot prune a record during delivery. Panic persistence cannot
await a lock: retention is queued after writing, so in-flight/concurrent writes
may briefly exceed the count, settling when the lock becomes available or on a
later load. Browser quota can prevent a new record even below the configured limits.

Web Locks requires a supporting browser in a secure context (HTTPS or localhost).
**Without a reliable Web Lock, recording and best-effort retention continue, but
delivery is deferred**, with a startup warning when a sink is configured.
A localStorage lease cannot safely prevent duplicate callbacks across tabs.
Use a supported secure context to drain those records. As on native, page death
after a sink commits but before acknowledgement can cause a retry; deduplicate
external effects by `record.id`.

Run the real Chrome/Wasm app regression suite with:

```sh
WASM_BINDGEN=/path/to/matching/wasm-bindgen node scripts/test-web-crash.mjs
```

It uses the existing browser CDP harness and a minimal `App` root. It makes no
rendered UI or GPU correctness claim. It covers panic/reload delivery,
competing tabs, failing Rust sinks, interrupted claims, privacy, UTF-8 JSON/count
bounds, console output, opt-out, throwing callbacks, quota errors and missing locks.

Segfaults, aborts, signal handlers, minidumps, vendor integrations and OpenTelemetry
export remain outside this implementation.
