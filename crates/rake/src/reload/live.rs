//! Native Blaze live-patch polling on the GPUI UI thread.
//!
//! This adapter is deliberately opt-in. It starts only for a development-controlled
//! native application with `RAKE_LIVE=blaze`; ordinary applications never reserve
//! patch space or start the Blaze watcher.

use std::time::Duration;

#[cfg(all(feature = "live", not(target_family = "wasm")))]
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[cfg(all(feature = "live", not(target_family = "wasm")))]
use std::{collections::BTreeMap, rc::Rc};

#[cfg(all(feature = "live", not(target_family = "wasm")))]
use blaze_live::Live;
#[cfg(all(feature = "live", not(target_family = "wasm")))]
use gpui::{App, Global, Task};
#[cfg(all(feature = "live", not(target_family = "wasm")))]
use serde_json::Value;

#[cfg(all(feature = "live", not(target_family = "wasm")))]
use crate::diagnostics::Collector;
#[cfg(all(feature = "live", not(target_family = "wasm")))]
use crate::reload::{ReloadError, ReloadStatus};
#[cfg(all(feature = "live", not(target_family = "wasm")))]
use crate::router::{Route, Router};

#[cfg(all(feature = "live", not(target_family = "wasm")))]
const TARGET: &str = "rake::reload::live";

/// How the host wants to treat a successful body patch.
///
/// `bypass` is the default because a body-only patch does not need to cancel managed
/// work. Full mode requires the existing runtime owner to run `prepare_reload` and
/// `finish_reload`; it never silently applies a body patch without that owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Boundary {
    Bypass,
    Full,
}

impl Boundary {
    #[cfg(all(feature = "live", not(target_family = "wasm")))]
    fn as_str(self) -> &'static str {
        match self {
            Self::Bypass => "bypass",
            Self::Full => "full",
        }
    }
}

/// The runtime event emitted after one UI-thread poll.
#[derive(Debug, PartialEq, Eq)]
enum RuntimeEvent {
    Applied {
        sequence: u64,
        functions: u64,
        statics: u64,
        restored: u64,
        took: Duration,
        waited: Duration,
        boundary: Boundary,
        boundary_finish_us: Option<u64>,
    },
    Restart {
        reason: String,
    },
    Declined {
        reason: String,
    },
}

/// The small, dependency-free outcome contract used by the mapper and its tests.
#[derive(Debug, PartialEq, Eq)]
enum LiveOutcome {
    Applied {
        sequence: u64,
        functions: u64,
        statics: u64,
        restored: u64,
        took: Duration,
        waited: Duration,
    },
    Restart(String),
    Declined(String),
}

#[cfg(all(feature = "live", not(target_family = "wasm")))]
impl From<blaze_live::Outcome> for LiveOutcome {
    fn from(outcome: blaze_live::Outcome) -> Self {
        match outcome {
            blaze_live::Outcome::Applied {
                sequence,
                functions,
                statics,
                restored,
                took,
                waited,
            } => Self::Applied {
                sequence,
                functions,
                statics,
                restored,
                took,
                waited,
            },
            blaze_live::Outcome::Restart(reason) => Self::Restart(reason),
            blaze_live::Outcome::Declined(reason) => Self::Declined(reason),
        }
    }
}

#[cfg(all(feature = "live", not(target_family = "wasm")))]
type FinishReloadCallback = dyn Fn(&str, bool) -> Result<ReloadStatus, ReloadError>;

#[cfg(all(feature = "live", not(target_family = "wasm")))]
#[derive(Clone)]
pub(crate) struct ReloadOwner {
    status: Rc<dyn Fn() -> ReloadStatus>,
    finish: Rc<FinishReloadCallback>,
}

#[cfg(all(feature = "live", not(target_family = "wasm")))]
impl ReloadOwner {
    pub(crate) fn from_router<R: Route>(router: Router<R>) -> Self {
        let status_router = router.clone();
        let finish_router = router;

        Self {
            status: Rc::new(move || status_router.reload_status()),
            finish: Rc::new(move |id, applied| finish_router.finish_reload(id, applied)),
        }
    }
}

#[cfg(all(feature = "live", not(target_family = "wasm")))]
fn boundary_from_environment() -> Boundary {
    boundary_from_value(std::env::var("RAKE_LIVE_BOUNDARY").ok().as_deref())
}

fn boundary_from_value(value: Option<&str>) -> Boundary {
    match value {
        Some("full") => Boundary::Full,
        _ => Boundary::Bypass,
    }
}

#[cfg(all(feature = "live", not(target_family = "wasm")))]
fn enabled() -> bool {
    std::env::var("RAKE_DEV_CONTROL").as_deref() == Ok("1")
        && std::env::var("RAKE_LIVE").as_deref() == Ok("blaze")
}

fn map_outcome(outcome: LiveOutcome, boundary: Boundary) -> RuntimeEvent {
    match outcome {
        LiveOutcome::Applied {
            sequence,
            functions,
            statics,
            restored,
            took,
            waited,
        } => RuntimeEvent::Applied {
            sequence,
            functions,
            statics,
            restored,
            took,
            waited,
            boundary,
            boundary_finish_us: None,
        },
        LiveOutcome::Restart(reason) => RuntimeEvent::Restart { reason },
        LiveOutcome::Declined(reason) => RuntimeEvent::Declined { reason },
    }
}

trait OutcomeSource {
    fn poll(&mut self) -> Option<LiveOutcome>;
}

#[cfg(all(feature = "live", not(target_family = "wasm")))]
struct LiveSource<'a>(&'a mut Live);

#[cfg(all(feature = "live", not(target_family = "wasm")))]
impl OutcomeSource for LiveSource<'_> {
    fn poll(&mut self) -> Option<LiveOutcome> {
        self.0.poll().map(Into::into)
    }
}

fn poll_source(source: &mut impl OutcomeSource, boundary: Boundary) -> Option<RuntimeEvent> {
    source.poll().map(|outcome| map_outcome(outcome, boundary))
}

#[cfg(all(feature = "live", not(target_family = "wasm")))]
fn poll_with_owner(
    source: &mut impl OutcomeSource,
    boundary: Boundary,
    reload_owner: Option<&ReloadOwner>,
) -> Option<RuntimeEvent> {
    let request_id = if boundary == Boundary::Full {
        let owner = reload_owner?;
        let status = (owner.status)();

        if status.phase != crate::reload::ReloadPhase::Ready {
            return None;
        }

        status.request.map(|request| request.id)?
    } else {
        String::new()
    };

    let mut event = poll_source(source, boundary);

    if boundary == Boundary::Full {
        let owner = reload_owner.expect("full live mode has a reload owner");
        if let Some(outcome) = &event {
            let applied = matches!(outcome, RuntimeEvent::Applied { .. });
            let started = Instant::now();

            if let Err(error) = (owner.finish)(&request_id, applied) {
                return Some(RuntimeEvent::Declined {
                    reason: format!("live boundary completion failed after patch outcome: {error}"),
                });
            }

            if let RuntimeEvent::Applied {
                boundary_finish_us, ..
            } = event.as_mut().expect("event was present")
            {
                *boundary_finish_us = Some(duration_us(started.elapsed()));
            }
        }
    }

    event
}

#[cfg(all(feature = "live", not(target_family = "wasm")))]
struct LiveDriver {
    live: Live,
    collector: Collector,
    boundary: Boundary,
    reload_owner: Option<ReloadOwner>,
    /// App-owned polling survives hidden, minimized and closed windows.
    _poll_task: Task<()>,
}

#[cfg(all(feature = "live", not(target_family = "wasm")))]
impl Global for LiveDriver {}

#[cfg(all(feature = "live", not(target_family = "wasm")))]
impl LiveDriver {
    fn poll(&mut self) -> Option<RuntimeEvent> {
        let mut source = LiveSource(&mut self.live);
        poll_with_owner(&mut source, self.boundary, self.reload_owner.as_ref())
    }

    fn record(&self, event: RuntimeEvent) {
        let mut fields = BTreeMap::new();
        fields.insert(
            "at_unix_us".into(),
            Value::from(duration_us(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default(),
            )),
        );

        let kind = match event {
            RuntimeEvent::Applied {
                sequence,
                functions,
                statics,
                restored,
                took,
                waited,
                boundary,
                boundary_finish_us,
            } => {
                fields.insert("sequence".into(), Value::from(sequence));
                fields.insert("functions".into(), Value::from(functions));
                fields.insert("statics".into(), Value::from(statics));
                fields.insert("restored".into(), Value::from(restored));
                fields.insert("took_us".into(), Value::from(duration_us(took)));
                fields.insert("waited_us".into(), Value::from(duration_us(waited)));
                fields.insert("boundary".into(), Value::from(boundary.as_str()));
                if let Some(duration) = boundary_finish_us {
                    fields.insert("boundary_finish_us".into(), Value::from(duration));
                }
                "live_patch_applied"
            }
            RuntimeEvent::Restart { reason } => {
                fields.insert("reason".into(), Value::from(reason));
                "live_restart_requested"
            }
            RuntimeEvent::Declined { reason } => {
                fields.insert("reason".into(), Value::from(reason));
                "live_patch_declined"
            }
        };

        self.collector
            .record("info", TARGET, kind, Vec::new(), fields);
    }
}

#[cfg(all(feature = "live", not(target_family = "wasm")))]
fn duration_us(duration: Duration) -> u64 {
    duration.as_micros().min(u128::from(u64::MAX)) as u64
}

/// Starts one app-owned UI timer, independent of window rendering or visibility.
#[cfg(all(feature = "live", not(target_family = "wasm")))]
pub(crate) fn install(collector: Collector, cx: &mut App, reload_owner: Option<ReloadOwner>) {
    if !enabled() {
        return;
    }

    if cx.try_global::<LiveDriver>().is_some() {
        return;
    }

    let boundary = boundary_from_environment();
    if boundary == Boundary::Full && reload_owner.is_none() {
        record_start_declined(
            &collector,
            "full reload boundary requires an attached runtime".into(),
            boundary,
        );
        return;
    }

    let live = match Live::start() {
        Ok(live) => live,
        Err(reason) => {
            record_start_declined(&collector, reason, boundary);
            return;
        }
    };

    let poll_task = start_polling(cx, poll_app);
    cx.set_global(LiveDriver {
        live,
        collector,
        boundary,
        reload_owner,
        _poll_task: poll_task,
    });
}

#[cfg(all(feature = "live", not(target_family = "wasm")))]
fn record_start_declined(collector: &Collector, reason: String, boundary: Boundary) {
    let mut fields = BTreeMap::new();
    fields.insert("reason".into(), Value::from(reason));
    fields.insert("boundary".into(), Value::from(boundary.as_str()));
    collector.record("warn", TARGET, "live_patch_declined", Vec::new(), fields);
}

#[cfg(all(feature = "live", not(target_family = "wasm")))]
fn start_polling(cx: &App, mut poll: impl FnMut(&mut App) + 'static) -> Task<()> {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
            cx.update(&mut poll);
        }
    })
}

#[cfg(all(feature = "live", not(target_family = "wasm")))]
fn poll_app(cx: &mut App) {
    let event = cx.global_mut::<LiveDriver>().poll();
    let applied = matches!(event, Some(RuntimeEvent::Applied { .. }));

    if let Some(event) = event {
        cx.global::<LiveDriver>().record(event);

        if applied {
            cx.refresh_windows();

            #[cfg(target_os = "macos")]
            for handle in cx.windows() {
                let _ = handle.update(cx, |_, window, cx| {
                    crate::application::request_frame(window, cx);
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(all(feature = "live", not(target_family = "wasm")))]
    use std::cell::RefCell;
    use std::collections::VecDeque;
    #[cfg(all(feature = "live", not(target_family = "wasm")))]
    use std::rc::Rc;

    use super::*;

    #[cfg(all(feature = "live", not(target_family = "wasm")))]
    use crate::reload::{ReloadMechanism, ReloadPhase, ReloadRequest};

    struct FakeSource {
        outcomes: VecDeque<LiveOutcome>,
        polls: usize,
    }

    impl OutcomeSource for FakeSource {
        fn poll(&mut self) -> Option<LiveOutcome> {
            self.polls += 1;
            self.outcomes.pop_front()
        }
    }

    #[test]
    fn fake_source_maps_applied_restart_and_declined_events() {
        let mut source = FakeSource {
            outcomes: VecDeque::from([
                LiveOutcome::Applied {
                    sequence: 7,
                    functions: 2,
                    statics: 1,
                    restored: 0,
                    took: Duration::from_micros(11),
                    waited: Duration::from_micros(13),
                },
                LiveOutcome::Restart("layout changed".into()),
                LiveOutcome::Declined("not enough room".into()),
            ]),
            polls: 0,
        };

        assert!(matches!(
            poll_source(&mut source, Boundary::Bypass),
            Some(RuntimeEvent::Applied {
                sequence: 7,
                boundary: Boundary::Bypass,
                ..
            })
        ));
        assert_eq!(
            poll_source(&mut source, Boundary::Full),
            Some(RuntimeEvent::Restart {
                reason: "layout changed".into(),
            })
        );
        assert_eq!(
            poll_source(&mut source, Boundary::Bypass),
            Some(RuntimeEvent::Declined {
                reason: "not enough room".into(),
            })
        );
        assert!(poll_source(&mut source, Boundary::Bypass).is_none());
    }

    #[cfg(all(
        feature = "live",
        feature = "view-test-support",
        not(target_family = "wasm")
    ))]
    #[test]
    fn ui_timer_polls_without_windows_or_frames_and_stops_with_its_owner() {
        let cx = gpui::TestAppContext::single();
        let source = Rc::new(RefCell::new(FakeSource {
            outcomes: VecDeque::new(),
            polls: 0,
        }));
        let events = Rc::new(RefCell::new(Vec::new()));
        let timer = cx.update(|cx| {
            let source = source.clone();
            let events = events.clone();

            start_polling(cx, move |_| {
                if let Some(event) = poll_source(&mut *source.borrow_mut(), Boundary::Bypass) {
                    events.borrow_mut().push(event);
                }
            })
        });
        cx.run_until_parked();

        for _ in 0..3 {
            cx.executor().advance_clock(Duration::from_millis(16));
            cx.run_until_parked();
        }

        assert_eq!(
            source.borrow().polls,
            3,
            "idle ticks must not require frames"
        );
        assert!(events.borrow().is_empty());
        source
            .borrow_mut()
            .outcomes
            .push_back(LiveOutcome::Applied {
                sequence: 9,
                functions: 1,
                statics: 0,
                restored: 0,
                took: Duration::from_micros(1),
                waited: Duration::from_micros(1),
            });
        cx.executor().advance_clock(Duration::from_millis(16));
        cx.run_until_parked();

        assert!(matches!(
            events.borrow().as_slice(),
            [RuntimeEvent::Applied { sequence: 9, .. }]
        ));
        assert!(
            cx.update(|cx| cx.windows().is_empty()),
            "no test window or frame was created"
        );
        drop(timer);
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        assert_eq!(
            source.borrow().polls,
            4,
            "dropping the owner must cancel polling"
        );
    }

    #[test]
    fn boundary_defaults_to_bypass_and_accepts_full() {
        assert_eq!(boundary_from_value(None), Boundary::Bypass);
        assert_eq!(boundary_from_value(Some("bypass")), Boundary::Bypass);
        assert_eq!(boundary_from_value(Some("full")), Boundary::Full);
        assert_eq!(boundary_from_value(Some("unknown")), Boundary::Bypass);
    }

    #[cfg(all(feature = "live", not(target_family = "wasm")))]
    fn fake_owner(
        phase: ReloadPhase,
        request: Option<&str>,
        finished: Rc<RefCell<Vec<(String, bool)>>>,
    ) -> ReloadOwner {
        let status = crate::reload::ReloadStatus {
            restored_from: None,
            generation: crate::router::GenerationId(1),
            phase,
            request: request.map(|id| ReloadRequest {
                id: id.into(),
                mechanism: ReloadMechanism::NativeCodeReplacement,
            }),
            error: None,
        };
        let status_for_read = status.clone();
        let status_for_finish = status;

        ReloadOwner {
            status: Rc::new(move || status_for_read.clone()),
            finish: Rc::new(move |id, applied| {
                finished.borrow_mut().push((id.into(), applied));
                Ok(status_for_finish.clone())
            }),
        }
    }

    #[cfg(all(feature = "live", not(target_family = "wasm")))]
    #[test]
    fn full_boundary_does_not_poll_before_ready() {
        let mut source = FakeSource {
            outcomes: VecDeque::from([LiveOutcome::Applied {
                sequence: 1,
                functions: 1,
                statics: 0,
                restored: 0,
                took: Duration::from_micros(1),
                waited: Duration::from_micros(1),
            }]),
            polls: 0,
        };
        let finished = Rc::new(RefCell::new(Vec::new()));
        let owner = fake_owner(ReloadPhase::Quiescing, Some("boundary-1"), finished.clone());

        assert!(poll_with_owner(&mut source, Boundary::Full, Some(&owner)).is_none());
        assert_eq!(source.polls, 0);
        assert!(finished.borrow().is_empty());
    }

    #[cfg(all(feature = "live", not(target_family = "wasm")))]
    #[test]
    fn full_boundary_finishes_each_real_outcome() {
        let mut source = FakeSource {
            outcomes: VecDeque::from([
                LiveOutcome::Applied {
                    sequence: 1,
                    functions: 1,
                    statics: 0,
                    restored: 0,
                    took: Duration::from_micros(1),
                    waited: Duration::from_micros(1),
                },
                LiveOutcome::Restart("layout changed".into()),
                LiveOutcome::Declined("not enough room".into()),
            ]),
            polls: 0,
        };
        let finished = Rc::new(RefCell::new(Vec::new()));
        let owner = fake_owner(ReloadPhase::Ready, Some("boundary-2"), finished.clone());

        assert!(matches!(
            poll_with_owner(&mut source, Boundary::Full, Some(&owner)),
            Some(RuntimeEvent::Applied {
                boundary: Boundary::Full,
                boundary_finish_us: Some(_),
                ..
            })
        ));
        assert!(matches!(
            poll_with_owner(&mut source, Boundary::Full, Some(&owner)),
            Some(RuntimeEvent::Restart { .. })
        ));
        assert!(matches!(
            poll_with_owner(&mut source, Boundary::Full, Some(&owner)),
            Some(RuntimeEvent::Declined { .. })
        ));
        assert_eq!(
            finished.borrow().as_slice(),
            &[
                ("boundary-2".into(), true),
                ("boundary-2".into(), false),
                ("boundary-2".into(), false),
            ]
        );
        assert_eq!(source.polls, 3);
    }
}
