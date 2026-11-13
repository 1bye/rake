use std::cell::Cell;
use std::rc::Rc;

use crate::gpui::{App, Global};

#[doc = "Rake-owned reactive allocations. GPUI caches and externally retained entities are excluded."]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct RuntimeStats {
    #[doc = "Mounted component owners."]
    pub components: usize,
    #[doc = "Registered signal entities."]
    pub signals: usize,
    #[doc = "Component-owned memo, effect, and resource observers."]
    pub observers: usize,
    #[doc = "Live tracked dependency subscriptions."]
    pub subscriptions: usize,
    #[doc = "Pending component-owned futures."]
    pub tasks: usize,
}

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Component,
    Signal,
    Observer,
    Subscription,
    Task,
}

#[derive(Default)]
struct Counters(Rc<Cell<RuntimeStats>>);
impl Global for Counters {}

pub(super) struct Lifetime {
    counts: Rc<Cell<RuntimeStats>>,
    kind: Kind,
}

impl Lifetime {
    pub fn new(cx: &mut App, kind: Kind) -> Self {
        let counts = cx.default_global::<Counters>().0.clone();
        let mut stats = counts.get();
        *field(&mut stats, kind) += 1;
        counts.set(stats);
        Self { counts, kind }
    }
}

impl Drop for Lifetime {
    fn drop(&mut self) {
        let mut stats = self.counts.get();
        *field(&mut stats, self.kind) -= 1;
        self.counts.set(stats);
    }
}

fn field(stats: &mut RuntimeStats, kind: Kind) -> &mut usize {
    match kind {
        Kind::Component => &mut stats.components,
        Kind::Signal => &mut stats.signals,
        Kind::Observer => &mut stats.observers,
        Kind::Subscription => &mut stats.subscriptions,
        Kind::Task => &mut stats.tasks,
    }
}

#[doc = "Reads diagnostic ownership counts; enabled only with view-test-support."]
pub fn runtime_stats(cx: &App) -> RuntimeStats {
    cx.try_global::<Counters>()
        .map_or_else(RuntimeStats::default, |counts| counts.0.get())
}
