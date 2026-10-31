# Chrome Performance

Performance records observed Rake/GPUI wall time through Chrome's bundled
frontend. It is not a native CPU sampler or GPU execution profiler. Recording
requires the opt-in `rake/devtools` feature and a mounted inspection host.

## Recording lifecycle

Record/Stop uses leased runtime capture, with polling independent of other enabled
CDP domains. A failed baseline releases the capture lease. Empty recordings include
session metadata and recording boundary markers. Stop completes with explicit loss
when a runtime disappears; the connection retains the completed stream for Chrome
to read before detaching. A checkpointed replacement completes the old recording
with loss instead of mixing identities and clocks into the new runtime. Stream
clients receive completion immediately. For Chrome’s `ReportEvents` mode the
finished data stays available and completion waits for the user’s **Stop**, so
Chrome’s active recording client can complete its handshake after replacement.

Each recording has a new stream handle. Starting another recording expires the
previous stream; old read/close requests cannot retarget or close the new one.
Disconnecting the client releases its recording lease. Runtime capture also expires
without renewal, so an ungraceful adapter exit cannot leave it enabled indefinitely.

## Frame analysis

Enable Chrome's **Show custom tracks** capture setting to see Rake's native tracks.
Draw tracks contain actual `Window::draw` durations. Redraw tracks show observed
invalidation-to-draw-end latency; unavailable latency is omitted, and a start before
the recording is visibly marked as clipped. Draws over the 16.67 ms reference
budget are colored red. This reference is not a detected monitor refresh deadline
or proof of a missed presentation frame.

Per-window recording summaries expose draw count, average/max draw duration,
invalidations, over-budget count and omitted callback observations. They are also
retained in exported metadata and `Rake.getPerformanceSummary`. Summary retention
is bounded to 64 windows, with an explicit omitted-windows flag. Browser FPS,
Core Web Vitals and GPU/presentation tracks are not fabricated.

## Element hotspots and Rust sources

Each retained callback appears on a phase-specific custom track and as a native
callback event in the main timeline. Details include stable boundary identity,
construction file/line/column and frame sequence. The callback event's source
presentation carries one real Rust construction location, not a JS stack or a
sampled Rust backtrace. Callback labels explicitly distinguish request-layout
(which may include rendering), prepaint and paint from browser engine work.

Hotspot summaries group by stable identity, label, phase and source and expose
sample count, maximum and average inclusive duration. They retain at most 128
groups / 64 KiB of grouping metadata and flag omissions. These are summaries of
retained observations, not all engine callbacks. Do not sum nested callback times
or treat a group as exclusive CPU time.

Captured source URLs use authenticated loopback endpoints. The server looks up the
exact runtime, retained frame and callback index, then applies the existing Rust
workspace confinement and 256 KiB source-file limit. Expired frames/runtimes or
unknown indices cannot read files. A bounded connection source cache also supports
Chrome's source-resource reads. Exported metadata keeps the Rust path/location;
live loopback links depend on the development session and retained source data.

## Commands, tasks, spans and HTTP

Opted-in runtimes supply a local calibration between the frame recording clock
and the diagnostic collector clock, including the sampling-window width and an
independent sequence high-water mark. Performance polls its own cursor: Console
Clear, Network pause and trace capture do not consume each other's observations.
Older runtimes without calibration retain frame recording but omit causal tracks.

Commands, tasks, tracing spans and HTTP lifecycles use separate custom tracks on
the same calibrated timeline. Details retain actual operation/task/span/parent IDs
and bounded diagnostic fields. Observed parent-span relationships also carry flow
links. Span lifetimes include async waiting and are not main-thread CPU slices.
HTTP lifetimes include application body consumption; time overlap with a draw does
not prove that the request caused it. No redraw causality is invented.

Work that started before the recording is marked and clipped to its beginning.
Pending observations at Stop are explicitly incomplete. Pending state is bounded
to 128 entries / 128 KiB, with retention loss reported. Diagnostic gaps and writer
losses mark a partial recording; eviction of already-consumed history does not.

## Retention, sessions and exports

The adapter retains at most 4 MiB of trace JSON; omitted events mark the recording
partial. Runtime frame retention remains independently bounded to 600 frames /
4 MiB, with at most 64 attributed callbacks per frame. Known callback omissions
are reported separately from lost frames. Repeated/overlapping pages do not
repeat observations, and eviction of already-consumed frames is not trace loss.
A changed recording clock or expired capture lease ends a partial recording.

Chrome's recent-session picker, Clear, Save trace and Load trace operate on the
completed profile. Chrome can compress its own export with gzip; the adapter's
`IO.read` stream is uncompressed JSON. Async custom entries use a separate trace
process, so their overlapping lifetimes and whole-recording summary do not inflate
Chrome's renderer main-thread totals. A separate **Capture status** lane
labels partial recordings and survives JSON export. Those totals still describe
instrumented wall time, not a sampled CPU profile.

## Verification and reproduction

The five delivery steps are recording lifecycle, frame analysis, callback/source
attribution, calibrated runtime work, and session/export hardening. Regression
coverage includes empty profiles, new/stale stream handles, UTF-8 chunking, the
trace byte ceiling, duplicate pages, unread gaps, clock changes and failed Stop.

```sh
cargo build -p rake-cli --offline
node scripts/test-performance-cdp.mjs
node scripts/test-performance-cdp.mjs --serve
```

The native harness needs macOS GUI access and Node 22+. It creates its own local
HTTP server, native supervisor and application. Its `performance-test` feature adds
a disposable canvas that sleeps for 25 ms during up to three actual paint callbacks,
plus a command, application task, tracing span and HTTP body read. The default run
checks source HTTP/resource loading, all causal lanes, partial capture on replacement
and crash, and reading a completed stream after the runtime has disappeared.
It does not control Chrome. `--serve` prints the stock Chrome URL and accepts
`work`, `capture`, `empty`, `restart` and `quit` on stdin; the app also has a
**Run performance workload** button. The fixture delay is not compiled into the
normal example.

Stock Chrome 156.0.8078.74 was checked on 2026-10-30 with **Show custom tracks** on:

- Record/Stop loaded a live native profile; the slow probe appeared at 26.04 ms.
- Search selected the callback and exposed stable identity, phase, inclusive
  measurement, construction location and an authenticated source link. Opening
  that link displayed the actual Rust fixture source.
- Imported JSON displayed draw, redraw, callback, command, task, span, HTTP and
  summary lanes. An imported 2.24-second profile showed about 345 ms of draw work,
  rather than counting async lifetimes in the main-thread total.
- A zero-draw profile loaded with a finite 0–37 ms range. After a live runtime
  replacement, Stop loaded the old runtime’s partial profile with a finite
  38.30-second range rather than waiting indefinitely.
- Save trace produced gzip JSON; Chrome loaded that export and its recent-session
  picker restored the earlier live recording. Clear returned to the live metrics page.

The focused CDP suite (73 tests), native GPUI recording/calibration test, diagnostic
and protocol tests, feature-enabled Clippy, production-disabled store tests,
rustdoc, formatting and the GPUI-enabled WASM compilation checks passed.
A separate no-default-features WASM check fails in the existing
`control/web.rs` references to GPUI-gated `reload::host`; those references predate
this work. This is not a claim of headless WASM support or a browser GPU runtime test.

## Remaining engine boundaries

Native CPU sampling and sampled Rust stacks, GPU execution/command buffers,
presentation/vsync deadlines, screenshots, memory allocation/heap sampling and
browser-only Web Vitals are unavailable. Chrome's **Record and reload** requires
browser navigation and is not a native application restart control. Capture
instrumentation affects measured timings. The existing compile-time `devtools`
feature keeps these inspection/capture hooks out of production builds.
