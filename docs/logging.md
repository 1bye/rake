# Application logging

`App` installs a default tracing subscriber. Use `rake::info!`, `warn!`,
`error!`, `debug!` and `trace!` as usual. Development builds write to stderr;
packaged (release) builds write private rotating files. Browser builds write to
the console and never use a native file or worker thread.

The existing `background` example emits a startup event. Build it in release mode
(`cargo run -p rake-app --release --example background`) to exercise file logging
on a native desktop; its development build emits to stderr.

## Paths and limits

For application ID `ID`, the current log is:

| Platform | Path |
| --- | --- |
| macOS | `~/Library/Logs/ID/logs/app.log` |
| Windows | `%LOCALAPPDATA%\ID\logs\app.log` |
| Linux | `$XDG_STATE_HOME/ID/logs/app.log`, default `~/.local/state/ID/logs/app.log` |

Roots must be absolute. Files are private (0600) and the application log directory
is private (0700) on Unix. Existing symlink files and nonprivate files/directories
are rejected using the same checks as crash storage. Keep the parent directory
trusted. Windows uses the user's inherited profile ACLs.

Each file is at most 1 MiB. Rotation retains `app.log.1` through `app.log.3`, for
a total budget of 4 MiB per application ID, plus an empty writer lock. A second
process using the same directory does not write while the first owns the lock.
No unrelated files are deleted. Existing oversized or unsafe logs disable output
for that launch rather than modifying untrusted files.

All file creation, locking, rotation and writes run on the existing diagnostics
worker. Producers use a queue of 1024 bounded metadata lines and never wait for
a disk write. A full queue drops output; `Snapshot::output_dropped` counts queue
loss. Missing roots, denied permissions, full disks and write failures discard
file output and do not stop the application. This is best-effort troubleshooting,
not an audit trail; queued lines can be lost on exit or panic.

## Configuration

The severity is selected in this order: a valid `RAKE_LOG` environment value,
`App::logging(LogConfig { level: Some(...), ..Default::default() })`, the
application-ID preference `log-level`, then INFO. Values are `off`, `error`,
`warn`, `info`, `debug` or `trace`, case insensitive. Invalid values fall back.
Preferences are sampled once at startup; changing them takes effect on relaunch.

```rust,ignore
let app = rake::App::new("example.app").logging(rake::logging::LogConfig {
    level: Some("warn".into()),
});
```

`App::without_logging()` leaves subscriber ownership to the host. The existing
`DiagnosticsConfig::subscriber(SubscriberMode::Host)` also supports replacement;
attach the shared collector layer to a custom subscriber if ctl diagnostics are
needed. Custom native output can use `DiagnosticsConfig::log_writer`; its factory
runs on the output worker. A host is responsible for its own output privacy and
retention. Logging and crash recording can be disabled independently.

## Read and tail a running app

Use the existing private ctl endpoint and runtime selection options:

```sh
rake ctl logs
rake ctl logs list --after 42
rake ctl logs --follow
# Equivalent: rake ctl logs follow
```

These commands query the running app through ctl, without opening arbitrary paths.
Replies are JSON snapshots with cumulative dropped/output-dropped counts and
structural records: sequence, elapsed microseconds, level and
kind. Follow advances the sequence cursor and suppresses empty replies, using the
existing ctl timeout. The retained tail is at most 128 records, also limited by
the configured collector capacity. Contention or evictions may leave gaps; this
is not a replay of older rotated files. `logs list` uses the same privacy policy.
Development `requests list` and `traces export` remain diagnostic tools and may
contain application-supplied fields.

## Privacy and crash reports

File logs and ctl logs use the crash-record privacy defaults: arbitrary messages,
fields, targets, span names, source paths and user content are omitted. File lines
contain elapsed microseconds, severity and the fixed kind `event`. Rake does not
collect environment values, credentials, request bodies or document content for
these logs. Development stderr and the browser console display tracing fields;
applications must avoid logging sensitive values there.

Native crash records optionally include `log_path` when the file worker opened
successfully. That path can contain a local account name. Log contents are never
copied into crash records or uploaded automatically. Sharing either artifact
requires the application's own consent and delivery policy.
