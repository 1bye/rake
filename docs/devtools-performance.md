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
with loss instead of mixing identities and clocks into the new runtime.

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
