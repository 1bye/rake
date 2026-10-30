# Chrome Console delivery plan

Each step is a separate commit. Stock Chrome is the supported frontend.

1. Log quality and source locations: implemented. Tracing events retain optional
   Rust file/line metadata, expandable fields and causal span context. Runtime
   delivery takes precedence over Log delivery, so enabling both does not duplicate
   a diagnostic. Captured Rust source links are read-only, confined to
   `RAKE_SOURCE_ROOT`, limited to 256 KiB and never enable a debugger.
2. Errors and panic/crash output: implemented. Evaluation failures use Chrome’s
   exception presentation and retain the request key for uncertain submissions.
   Supervisor stderr and exit status are drained even after runtime IPC fails.
   Development launches default to `RUST_BACKTRACE=1` (an explicit user value wins).
   Native panic headers/backtraces are retained as process text; no panic hook,
   recovery mechanism or debugger is installed.
3. Command discovery, completion and operation results: pending.
4. Clear, preserve-log lifecycle and retained objects: pending.
5. Stock Chrome verification and repeatable native/CDP checks: pending.

Source retention is bounded to 128 paths and eight concurrent source streams per
connection. Source locations are unavailable for older runtimes and manual records
without call-site metadata. Existing Rust source editor opening remains available.
