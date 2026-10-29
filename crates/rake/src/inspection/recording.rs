//! Leased recording of actual GPUI draws and inclusive inspection callback timings.

#[cfg(feature = "gpui")]
use super::NodeTiming;
use super::{FrameTiming, Frames, InspectionError, Store};
#[cfg(any(feature = "gpui", test))]
use crate::router::WindowId;
use std::collections::VecDeque;
#[cfg(feature = "gpui")]
use std::time::Duration;
use web_time::Instant;

#[cfg(any(feature = "gpui", test))]
const MAX_FRAMES: usize = 600;
#[cfg(any(feature = "gpui", test))]
const MAX_BYTES: usize = 4 * 1024 * 1024;
#[cfg(feature = "gpui")]
const MAX_NODE_TIMINGS: usize = 64;

pub(super) struct Recording {
    pub until: Option<Instant>,
    #[cfg(feature = "gpui")]
    epoch: Instant,
    frames: VecDeque<(FrameTiming, usize)>,
    #[cfg(any(feature = "gpui", test))]
    bytes: usize,
    sequence: u64,
    dropped: u64,
    #[cfg(feature = "gpui")]
    collector: Option<gpui::FrameTimingCollector>,
    #[cfg(feature = "gpui")]
    guard: Option<FrameTraceGuard>,
    #[cfg(feature = "gpui")]
    windows: Vec<(WindowId, gpui::WindowId)>,
    #[cfg(feature = "gpui")]
    pending: VecDeque<Pending>,
}

impl Default for Recording {
    fn default() -> Self {
        Self {
            until: None,
            #[cfg(feature = "gpui")]
            epoch: Instant::now(),
            frames: VecDeque::new(),
            #[cfg(any(feature = "gpui", test))]
            bytes: 0,
            sequence: 0,
            dropped: 0,
            #[cfg(feature = "gpui")]
            collector: None,
            #[cfg(feature = "gpui")]
            guard: None,
            #[cfg(feature = "gpui")]
            windows: Vec::new(),
            #[cfg(feature = "gpui")]
            pending: VecDeque::new(),
        }
    }
}

impl Store {
    pub(crate) fn record_frames(
        &self,
        enabled: bool,
        lease_ms: u64,
    ) -> Result<(), InspectionError> {
        if enabled && !(1..=5000).contains(&lease_ms) {
            return Err(InspectionError::InvalidRecordingLease);
        }
        if self.status() == super::Status::Unavailable {
            return Err(InspectionError::RecordingUnavailable);
        }
        #[cfg(feature = "gpui")]
        {
            let mut state = self.0.borrow_mut();
            let recording = &mut state.frames;
            recording.collect();
            let was_active = recording.active();
            if enabled {
                if !recording.active() {
                    recording.dropped += recording.frames.len() as u64;
                    recording.frames.clear();
                    recording.bytes = 0;
                    recording.pending.clear();
                    recording.epoch = Instant::now();
                    recording.guard = Some(FrameTraceGuard::acquire());
                    recording.collector = Some(gpui::FrameTimingCollector::new());
                }
                recording.until = Some(Instant::now() + Duration::from_millis(lease_ms));
            } else {
                recording.stop();
            }
            if was_active != enabled {
                state.changed.notify(usize::MAX);
            }
            Ok(())
        }
        #[cfg(not(feature = "gpui"))]
        Err(InspectionError::RecordingUnavailable)
    }

    pub(crate) fn frames(&self, after: u64) -> Frames {
        let mut state = self.0.borrow_mut();
        let recording = &mut state.frames;
        #[cfg(feature = "gpui")]
        recording.collect();
        recording.expire(Instant::now());
        let mut frames = Vec::new();
        let mut bytes = 4096;
        for (frame, size) in recording
            .frames
            .iter()
            .filter(|(frame, _)| frame.sequence > after)
        {
            if frames.len() >= 20 || bytes + size >= super::MAX_SNAPSHOT_BYTES {
                break;
            }
            bytes += size;
            frames.push(frame.clone());
        }
        let cursor = frames
            .last()
            .map_or(after.min(recording.sequence), |frame| frame.sequence);
        Frames {
            recording: recording.until.is_some(),
            more: recording
                .frames
                .back()
                .is_some_and(|(frame, _)| frame.sequence > cursor),
            gap: after > 0
                && recording
                    .frames
                    .front()
                    .is_some_and(|(frame, _)| frame.sequence > after.saturating_add(1)),
            dropped: recording.dropped,
            frames,
            cursor,
        }
    }
}

impl Recording {
    pub fn expire(&mut self, now: Instant) {
        if self.until.is_some_and(|until| until <= now) {
            #[cfg(feature = "gpui")]
            self.collect();
            self.stop();
        }
    }

    fn stop(&mut self) {
        self.until = None;
        #[cfg(feature = "gpui")]
        {
            self.collector = None;
            self.guard = None;
            self.pending.clear();
        }
    }

    #[cfg(feature = "gpui")]
    pub fn detach(&mut self, window: WindowId) {
        self.windows.retain(|(id, _)| *id != window);
        self.pending.retain(|pending| pending.window != window);
        if self.windows.is_empty() {
            self.stop();
        }
    }

    #[cfg(feature = "gpui")]
    pub fn active(&self) -> bool {
        self.until.is_some_and(|until| until > Instant::now())
    }

    #[cfg(feature = "gpui")]
    pub fn begin(&mut self, window: WindowId, gpui_window: gpui::WindowId) {
        if !self.windows.iter().any(|(id, _)| *id == window) {
            self.windows.push((window, gpui_window));
        }
        self.expire(Instant::now());
        if self.active() {
            if self.pending.len() == 64 {
                self.pending.pop_front();
            }
            self.pending.push_back(Pending {
                window,
                nodes: Vec::new(),
                observed: 0,
            });
        }
    }

    #[cfg(feature = "gpui")]
    fn node(&mut self, window: WindowId, node: NodeTiming) {
        if !self.active() {
            return;
        }
        let Some(pending) = self
            .pending
            .iter_mut()
            .rev()
            .find(|pending| pending.window == window)
        else {
            return;
        };
        pending.observed += 1;
        if pending.nodes.len() < MAX_NODE_TIMINGS {
            pending.nodes.push(node);
        } else if let Some((index, least)) = pending
            .nodes
            .iter()
            .enumerate()
            .min_by_key(|(_, sample)| sample.duration_us)
            && node.duration_us > least.duration_us
        {
            pending.nodes[index] = node;
        }
    }

    #[cfg(feature = "gpui")]
    fn collect(&mut self) {
        let Some(collector) = &mut self.collector else {
            return;
        };
        for timing in collector.collect_unseen() {
            let Some(window) = self
                .windows
                .iter()
                .find(|(_, id)| *id == timing.window_id)
                .map(|(id, _)| *id)
            else {
                continue;
            };
            let pending = self
                .pending
                .iter()
                .position(|pending| pending.window == window)
                .and_then(|index| self.pending.remove(index));
            let (mut nodes, observed) =
                pending.map_or((Vec::new(), 0), |pending| (pending.nodes, pending.observed));
            nodes.sort_by_key(|node| std::cmp::Reverse(node.duration_us));
            self.sequence = self.sequence.saturating_add(1);
            let duration = micros(timing.draw_duration());
            let elapsed = micros(self.epoch.elapsed())
                .saturating_sub(micros(timing.draw_end.elapsed()))
                .saturating_sub(duration);
            self.push(FrameTiming {
                sequence: self.sequence,
                window,
                elapsed_us: elapsed,
                draw_us: duration,
                dirty_to_draw_us: timing.dirty_to_draw_duration().map(micros),
                invalidations: timing.invalidations,
                omitted_nodes: observed.saturating_sub(nodes.len()),
                nodes,
            });
        }
    }

    #[cfg(any(feature = "gpui", test))]
    fn push(&mut self, frame: FrameTiming) {
        self.sequence = self.sequence.max(frame.sequence);
        let bytes = serde_json::to_vec(&frame).map_or(MAX_BYTES, |bytes| bytes.len());
        if bytes >= super::MAX_SNAPSHOT_BYTES - 4096 {
            self.dropped += 1;
            return;
        }
        while self.frames.len() >= MAX_FRAMES || self.bytes + bytes > MAX_BYTES {
            if let Some((_, removed)) = self.frames.pop_front() {
                self.bytes -= removed;
                self.dropped += 1;
            } else {
                break;
            }
        }
        self.bytes += bytes;
        self.frames.push_back((frame, bytes));
    }
}

#[cfg(feature = "gpui")]
struct Pending {
    window: WindowId,
    nodes: Vec<NodeTiming>,
    observed: usize,
}

#[cfg(feature = "gpui")]
#[derive(Clone)]
pub(super) struct Target {
    pub store: Store,
    pub window: WindowId,
    pub stable: Option<String>,
    pub name: String,
    pub source: super::SourceLocation,
}

#[cfg(feature = "gpui")]
impl Target {
    pub fn start(&self, phase: super::FramePhase) -> Option<Timer> {
        let state = self.store.0.borrow();
        state.frames.active().then(|| Timer {
            target: self.clone(),
            phase,
            start: Instant::now(),
            elapsed: micros(state.frames.epoch.elapsed()),
        })
    }
}

#[cfg(feature = "gpui")]
pub(super) struct Timer {
    target: Target,
    phase: super::FramePhase,
    start: Instant,
    elapsed: u64,
}
#[cfg(feature = "gpui")]
impl Timer {
    pub fn finish(self) {
        let duration = micros(self.start.elapsed());
        self.target.store.0.borrow_mut().frames.node(
            self.target.window,
            NodeTiming {
                stable_id: self.target.stable,
                name: self.target.name,
                source: self.target.source,
                phase: self.phase,
                elapsed_us: self.elapsed,
                duration_us: duration,
            },
        );
    }
}

#[cfg(feature = "gpui")]
fn micros(duration: Duration) -> u64 {
    duration.as_micros().min(u64::MAX as u128) as u64
}

// GPUI exposes one process-wide switch. Nested Rake hosts restore its prior state
// only when the last Rake recorder releases it; an already enabled profiler stays on.
#[cfg(feature = "gpui")]
struct FrameTraceGuard;
#[cfg(feature = "gpui")]
static TRACE_USERS: std::sync::Mutex<(usize, bool)> = std::sync::Mutex::new((0, false));
#[cfg(feature = "gpui")]
impl FrameTraceGuard {
    fn acquire() -> Self {
        let mut users = TRACE_USERS.lock().expect("frame recorder lock poisoned");
        if users.0 == 0 {
            users.1 = gpui::set_frame_trace_enabled(true);
        }
        users.0 += 1;
        Self
    }
}
#[cfg(feature = "gpui")]
impl Drop for FrameTraceGuard {
    fn drop(&mut self) {
        let mut users = TRACE_USERS.lock().expect("frame recorder lock poisoned");
        users.0 -= 1;
        if users.0 == 0 && users.1 {
            gpui::set_frame_trace_enabled(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_pages_are_bounded_and_report_eviction_and_lease_expiry() {
        let store = Store::default();
        for sequence in 1..=700 {
            store.0.borrow_mut().frames.push(FrameTiming {
                sequence,
                window: WindowId(1),
                elapsed_us: sequence * 10,
                draw_us: 4,
                dirty_to_draw_us: None,
                invalidations: 1,
                nodes: Vec::new(),
                omitted_nodes: 0,
            });
        }
        let page = store.frames(1);
        assert!(page.gap);
        assert!(page.more);
        assert_eq!(page.dropped, 100);
        assert_eq!(page.frames[0].sequence, 101);
        assert!(serde_json::to_vec(&page).unwrap().len() < super::super::MAX_SNAPSHOT_BYTES);
        let mut state = store.0.borrow_mut();
        state.frames.until = Some(Instant::now());
        state.frames.expire(Instant::now());
        assert!(state.frames.until.is_none());
    }
}
