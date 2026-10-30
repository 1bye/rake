# Chrome Console

All five Console steps are implemented as separate commits. Stock Chrome is the
supported frontend; native/CDP tests supplement the Chrome UI checks.

## Supported workflows

1. **Logs and sources.** Tracing events retain Rust file/line metadata, structured
   fields, severity and causal span context. One structured event is delivered
   when Log and Runtime are both enabled. Supervised development output has an
   explicit marker for mirrored diagnostics; mirrors are hidden while IPC is live
   and retained as fallback after failure. Raw stdout/stderr remain visible.
   Log source links open read-only Rust text in Chrome through authenticated
   loopback URLs. The adapter checks the exact runtime and retained diagnostic,
   confines reads to `RAKE_SOURCE_ROOT`, and caps a file at 256 KiB. Expired
   diagnostics/runtimes and paths outside the root cannot be used to read files.
2. **Failures and crashes.** Evaluation errors use Chrome’s exception presentation.
   Command results explicitly distinguish committed, running, queued, awaiting
   decision, failed, blocked and cancelled outcomes. Supervisor output is drained
   after IPC fails, including panic headers, available backtraces and exit status.
   Development launches default to `RUST_BACKTRACE=1`; explicit user values win.
3. **Command usability.** `rake.help()` lists helpers, `rake.commands()` returns
   registered command schemas, and `rake.command("demo.echo")` describes one.
   The `rake` namespace is reserved for DevTools helpers. Namespace properties
   provide read-only completion data. Results include a
   request key and an `observe` token. Pass the token as literal JSON to
   `rake.operation({"runtimeId":"…","id":"op-1"})` to observe without submitting
   again. The UUID must match the current runtime, so an old token never retargets.
   Standalone inspection hosts provide helpers and an empty application command
   list, rather than inventing a command registry.
4. **Lifecycle.** Clear advances this connection’s diagnostic/output cursors and
   releases Console objects without deleting application diagnostics. Nested
   objects inherit their group; releasing one group leaves other groups intact.
   Checkpointed replacement signals a new virtual frame/context for Chrome’s
   Keep log setting. Old object handles expire instead of selecting new data.
   Objects may also expire through bounded retention, explicit release or Clear.
5. **Verification.** A disposable native routing fixture and the actual supervisor
   exercise the workflows, including deliberate failure, a slow command, a
   checkpointed restart and a real panic followed by exit code 73.

Diagnostic source fields, capture and mirror state compile out without
`rake/devtools`. The CLI enables the diagnostics wire feature so it can read
metadata from opted-in runtimes. There is no Rust/JavaScript evaluator, panic
recovery hook or Rust debugger. Backtraces are supervised process text, not
invented JavaScript stack frames.

## Stock Chrome evidence

On 2026-10-30, Google Chrome **156.0.8078.74** and the native `rake-routing`
`console-test` fixture were exercised using Chrome’s bundled DevTools UI:

- Reviewed commands entered through the Console prompt; helper namespace
  suggestions and expandable help/results appeared.
- Information, warning and error rows displayed their correct severity; Unicode
  command results and structured/causal fields were exposed for expansion.
- Clicking a log’s source link opened the real Rust file as read-only text.
- A failed result displayed `status: failed`; slow commands displayed
  `status: running` with a request key and observation token.
- Search found nine matches; Clear reported zero messages.
- Keep log retained previous failed/running results across a checkpointed restart
  and displayed the successor’s startup plus the restored-from notice.
- After clearing, the deliberate crash left the panic message, Rust backtrace,
  exit code 73 and disconnect notice visible in Chrome.

Chrome’s paste protection remained enabled. Console command entry used reviewed
keyboard input. Rust source reads, nested object/group expiry, old operation
rejection, standalone discovery and the delivery boundaries also have native/CDP
or focused regression coverage. WASM evidence is compilation, not browser/GPU
execution.

## Repeatable checks

```sh
cargo build -p rake-cli -p rake-routing --features rake-routing/console-test --offline
node scripts/test-console-cdp.mjs
node scripts/test-console-cdp.mjs --serve
```

The default command starts an isolated supervisor, checks real native/CDP
behavior and stops its own processes. It also verifies source links across replacement
when `--instance` originally selected the old runtime UUID. It creates one uniquely
named asset to trigger checkpointed replacement and removes that owned asset and
its isolated session/build directory during cleanup.
`--serve` prints a stock Chrome URL for repeating the UI pass; stop it with Ctrl-C.
This requires macOS GUI access and Node 22+ and fails explicitly when unavailable.
The fixture’s `test.console({"mode":"log|fail|slow|crash"})` commands exist only
with the example’s opt-in `console-test` feature; `crash` terminates that fixture.

Source retention is bounded to 128 URLs and eight concurrent source streams per
connection. Older runtimes and manually recorded events may have no source data.
Independent same-name processes are not treated as checkpointed successors.
