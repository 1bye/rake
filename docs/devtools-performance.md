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
