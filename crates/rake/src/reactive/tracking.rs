use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::gpui::{AnyWeakEntity, App, AppContext, Context, Entity, Global, Subscription};

use super::signal::{Key, Signal};

pub(super) struct Dependency {
    key: Key,
    subscribe: fn(Key, AnyWeakEntity, &mut App) -> Option<Subscription>,
}

#[derive(Clone, Default)]
struct Tracking(Rc<RefCell<Vec<Vec<Dependency>>>>);
impl Global for Tracking {}

struct Scope(Tracking);
impl Drop for Scope {
    fn drop(&mut self) {
        self.0.0.borrow_mut().pop();
    }
}

pub(super) fn collect<R>(cx: &mut App, read: impl FnOnce(&mut App) -> R) -> (R, Vec<Dependency>) {
    let tracking = cx.default_global::<Tracking>().clone();
    tracking.0.borrow_mut().push(Vec::new());
    let scope = Scope(tracking);
    let value = read(cx);
    let dependencies = std::mem::take(scope.0.0.borrow_mut().last_mut().unwrap());
    (value, dependencies)
}

pub(super) fn record<T: 'static>(signal: Signal<T>, cx: &App) {
    let Some(tracking) = cx.try_global::<Tracking>() else {
        return;
    };
    let mut stack = tracking.0.borrow_mut();
    let Some(dependencies) = stack.last_mut() else {
        return;
    };
    dependencies.push(Dependency {
        key: signal.key,
        subscribe: |key, target, cx| {
            let signal = Signal::<T>::from_key(key);
            let entity = signal.resolve(cx)?.clone();
            Some(cx.observe(&entity, move |_, cx| {
                if let Some(target) = target.upgrade() {
                    cx.notify(target.entity_id());
                }
            }))
        },
    });
}

#[derive(Default)]
pub(super) struct Subscriptions {
    entries: HashMap<Key, SubscriptionEntry>,
    revision: u64,
}

struct SubscriptionEntry {
    _subscription: Subscription,
    #[cfg(feature = "view-test-support")]
    _lifetime: super::stats::Lifetime,
    revision: u64,
}

impl Subscriptions {
    pub fn replace(&mut self, dependencies: Vec<Dependency>, target: AnyWeakEntity, cx: &mut App) {
        self.revision = self.revision.wrapping_add(1);
        for dependency in dependencies {
            match self.entries.entry(dependency.key) {
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    entry.get_mut().revision = self.revision
                }
                std::collections::hash_map::Entry::Vacant(entry) => {
                    if let Some(subscription) =
                        (dependency.subscribe)(dependency.key, target.clone(), cx)
                    {
                        entry.insert(SubscriptionEntry {
                            _subscription: subscription,
                            revision: self.revision,
                            #[cfg(feature = "view-test-support")]
                            _lifetime: super::stats::Lifetime::new(
                                cx,
                                super::stats::Kind::Subscription,
                            ),
                        });
                    }
                }
            }
        }
        self.entries
            .retain(|_, entry| entry.revision == self.revision);
    }
}

pub(super) struct Observer<F> {
    run: F,
    subscriptions: Subscriptions,
}

impl<F: FnMut(&mut App) + 'static> Observer<F> {
    pub(super) fn new(cx: &mut App, run: F, dependencies: Vec<Dependency>) -> Entity<Self> {
        cx.new(|cx| {
            cx.observe_self(|this: &mut Self, cx| this.refresh(cx))
                .detach();
            let mut subscriptions = Subscriptions::default();
            subscriptions.replace(dependencies, cx.entity().into_any().downgrade(), cx);
            Self { run, subscriptions }
        })
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let (_, dependencies) = collect(cx, |cx| (self.run)(cx));
        self.subscriptions
            .replace(dependencies, cx.entity().into_any().downgrade(), cx);
    }
}
