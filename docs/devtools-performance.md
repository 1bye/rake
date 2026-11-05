# Chrome Performance

Performance records observed Rake/GPUI wall time through Chrome's bundled
frontend. It is not a native CPU sampler or GPU execution profiler. Recording
requires the opt-in `rake/devtools` feature and a mounted inspection host.

## Recording lifecycle

Record and Stop use leased runtime capture, polling independently of other enabled CDP domains. A failed baseline releases the lease, and empty recordings still include session metadata and boundary markers.

- Stop completes with explicit loss when a runtime disappears, and the connection keeps the completed stream for Chrome to read.
- A checkpointed replacement completes the old recording with loss, instead of mixing identities and clocks.
- Stream clients get completion immediately. In Chrome's `ReportEvents` mode, completion waits for your **Stop**, so Chrome can finish its handshake after replacement.

Each recording has a new stream handle. Starting another recording expires the
previous stream; old read/close requests cannot retarget or close the new one.
Disconnecting the client releases its recording lease. Runtime capture also expires
without renewal, so an ungraceful adapter exit cannot leave it enabled indefinitely.

## Frame analysis

Enable Chrome's **Show custom tracks** capture setting to see Rake's native tracks.

- **Draw tracks** show actual `Window::draw` durations. Draws over the 16.67 ms reference budget are red. That's a reference, not a detected refresh deadline or proof of a missed frame.
- **Redraw tracks** show observed invalidation-to-draw-end latency. Unavailable latency is omitted, and a start before the recording is marked as clipped.

Per-window recording summaries expose draw count, average/max draw duration,
invalidations, over-budget count and omitted callback observations. They are also
retained in exported metadata and `Rake.getPerformanceSummary`. Summary retention
is bounded to 64 windows, with an explicit omitted-windows flag. Browser FPS,
Core Web Vitals and GPU/presentation tracks are not fabricated.

## Element hotspots and Rust sources

Each retained callback appears on a phase-specific custom track and as a native callback event in the main timeline. Details include stable boundary identity, construction file, line and column, and frame sequence.

The source presentation carries one real Rust construction location, not a JS stack or sampled backtrace. Labels distinguish request-layout (which may include rendering), prepaint and paint from browser engine work.

Hotspot summaries group by stable identity, label, phase and source, and show sample count, maximum and average inclusive duration. They keep at most 128 groups and 64 KiB of grouping metadata, and flag omissions.

These summarize retained observations, not all engine callbacks. Don't sum nested callback times or read a group as exclusive CPU time.

Captured source URLs use authenticated loopback endpoints. The server looks up the exact runtime, retained frame and callback index, then applies Rust workspace confinement and the 256 KiB file limit. Expired frames or runtimes and unknown indices can't read files.

A bounded connection cache also supports Chrome's source-resource reads. Exported metadata keeps the Rust path and location, while live links depend on the development session and retained source data.

## Commands, tasks, spans and HTTP

Opted-in runtimes supply a local calibration between the frame recording clock
and the diagnostic collector clock, including the sampling-window width and an
independent sequence high-water mark. Performance polls its own cursor: Console
Clear, Network pause and trace capture do not consume each other's observations.
Older runtimes without calibration retain frame recording but omit causal tracks.

Commands, tasks, tracing spans and HTTP lifecycles use separate custom tracks on the same calibrated timeline. Details keep actual operation, task, span and parent IDs and bounded fields, and observed parent-span relationships carry flow links.

Span lifetimes include async waiting, so they aren't main-thread CPU slices. HTTP lifetimes include body consumption, and overlap with a draw doesn't prove the request caused it. No redraw causality is invented.

Work that started before the recording is marked and clipped to its beginning.
Pending observations at Stop are explicitly incomplete. Pending state is bounded
to 128 entries / 128 KiB, with retention loss reported. Diagnostic gaps and writer
losses mark a partial recording; eviction of already-consumed history does not.

## Retention, sessions and exports

The adapter keeps at most 4 MiB of trace JSON, and omitted events mark the recording partial. Runtime frame retention is separately bounded to 600 frames or 4 MiB, with at most 64 attributed callbacks per frame. Known callback omissions are reported separately from lost frames.

Overlapping pages don't repeat observations, and evicting already-consumed frames isn't trace loss. A changed recording clock or expired capture lease ends a partial recording.

Chrome's recent-session picker, Clear, Save trace and Load trace work on the completed profile. Chrome can gzip its own export, while the adapter's `IO.read` stream is uncompressed JSON.

Async custom entries use a separate trace process, so overlapping lifetimes don't inflate Chrome's renderer main-thread totals. Those totals still describe instrumented wall time, not a sampled CPU profile. A **Capture status** lane labels partial recordings and survives JSON export.

## Verification and reproduction

The five delivery steps are recording lifecycle, frame analysis, callback/source
attribution, calibrated runtime work, and session/export hardening. Regression
coverage includes empty profiles, new/stale stream handles, UTF-8 chunking, the
trace byte ceiling, duplicate pages, unread gaps, clock changes and failed Stop.

```sh
cargo build -p rake-app-cli --offline
node scripts/test-performance-cdp.mjs
node scripts/test-performance-cdp.mjs --serve
```

The native harness needs macOS GUI access and Node 22+. It creates its own HTTP server, supervisor and app, and doesn't control Chrome. Its `performance-test` feature adds a disposable canvas that sleeps 25 ms during up to three paint callbacks, plus a command, task, span and HTTP body read. That delay isn't compiled into the normal example.

For a manual pass, `--serve` prints the stock Chrome URL and accepts `work`, `capture`, `empty`, `restart` and `quit` on stdin. The app also has a **Run performance workload** button.

Stock Chrome 156.0.8078.74 was checked on 2026-10-30 with **Show custom tracks** on:

See [test coverage](internal/test-coverage.md) for what's been checked in stock Chrome.

## Remaining engine boundaries

Native CPU sampling and sampled Rust stacks, GPU execution/command buffers,
presentation/vsync deadlines, screenshots, memory allocation/heap sampling and
browser-only Web Vitals are unavailable. Chrome's **Record and reload** requires
browser navigation and is not a native application restart control. Capture
instrumentation affects measured timings. The existing compile-time `devtools`
feature keeps these inspection/capture hooks out of production builds.
