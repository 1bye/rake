use std::marker::PhantomData;
use std::rc::Rc;

use slotmap::{DefaultKey, SlotMap};
use uuid::Uuid;

use crate::gpui::{App, AppContext, Entity, Global};

pub(super) struct Store<T: 'static> {
    identity: Uuid,
    values: SlotMap<DefaultKey, Entry<T>>,
}

struct Entry<T: 'static> {
    entity: Entity<T>,
    #[cfg(feature = "view-test-support")]
    _lifetime: super::stats::Lifetime,
}

impl<T: 'static> Global for Store<T> {}

impl<T: 'static> Default for Store<T> {
    fn default() -> Self {
        Self {
            identity: Uuid::new_v4(),
            values: SlotMap::with_key(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct Key {
    pub registry: Uuid,
    pub slot: DefaultKey,
}

#[doc = "A copyable, application-local handle to a typed GPUI entity."]
pub struct Signal<T: 'static> {
    pub(super) key: Key,
    value: PhantomData<fn() -> T>,
    thread: PhantomData<Rc<()>>,
}

impl<T> Copy for Signal<T> {}
impl<T> Clone for Signal<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> PartialEq for Signal<T> {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}
impl<T> Eq for Signal<T> {}

impl<T: Clone + 'static> super::Value<T> for Signal<T> {
    fn resolve(self, cx: &App) -> T {
        self.get(cx)
    }
}

impl<T: 'static> Signal<T> {
    #[doc = "Creates application-owned state. Setup::signal ties its lifetime to a component instead."]
    pub fn new(cx: &mut App, value: T) -> Self {
        let entity = cx.new(|_| value);
        Self::from_entity(cx, entity)
    }

    #[doc = "Registers an existing entity with this application without copying its value."]
    pub fn from_entity(cx: &mut App, entity: Entity<T>) -> Self {
        let entry = Entry {
            entity,
            #[cfg(feature = "view-test-support")]
            _lifetime: super::stats::Lifetime::new(cx, super::stats::Kind::Signal),
        };
        let store = cx.default_global::<Store<T>>();
        Self::from_key(Key {
            registry: store.identity,
            slot: store.values.insert(entry),
        })
    }

    pub(super) fn from_key(key: Key) -> Self {
        Self {
            key,
            value: PhantomData,
            thread: PhantomData,
        }
    }

    pub(super) fn resolve(self, cx: &App) -> Option<&Entity<T>> {
        let store = cx.try_global::<Store<T>>()?;
        (store.identity == self.key.registry)
            .then(|| store.values.get(self.key.slot).map(|entry| &entry.entity))
            .flatten()
    }

    #[doc = "Borrows the current value and records the read through GPUI. Panics after disposal."]
    pub fn read(self, cx: &App) -> &T {
        super::tracking::record(self, cx);
        self.resolve(cx)
            .expect("signal has been disposed or belongs to another application")
            .read(cx)
    }

    #[doc = "Clones the current value, tracking the same dependency as read."]
    pub fn get(self, cx: &App) -> T
    where
        T: Clone,
    {
        self.read(cx).clone()
    }

    #[doc = "Reads without cloning the stored value."]
    pub fn with<R>(self, cx: &App, read: impl FnOnce(&T) -> R) -> R {
        read(self.read(cx))
    }

    #[doc = "Returns the underlying GPUI entity. Panics after disposal."]
    pub fn entity(self, cx: &App) -> Entity<T> {
        self.resolve(cx)
            .expect("signal has been disposed or belongs to another application")
            .clone()
    }

    #[doc = "Replaces the value and notifies its readers. Disposed signals do nothing."]
    pub fn set<C: super::SignalContext>(self, cx: &mut C, value: T) {
        self.update(cx, |current| *current = value);
    }

    #[doc = "Replaces and notifies only when the value differs. Disposed signals do nothing."]
    pub fn set_if_changed<C: super::SignalContext>(self, cx: &mut C, value: T)
    where
        T: PartialEq,
    {
        let entity = cx.signal_entity(self);
        if let Some(entity) = entity {
            entity.update(cx, |current, cx| {
                if *current != value {
                    *current = value;
                    cx.notify();
                }
            });
        }
    }

    #[doc(hidden)]
    pub fn set_prop(self, cx: &mut App, value: T)
    where
        T: PartialEq,
    {
        let Some(entity) = self.resolve(cx).cloned() else {
            return;
        };
        let changed = entity.update(cx, |current, _| {
            if *current == value {
                return false;
            }
            *current = value;
            true
        });
        if changed {
            cx.defer(move |cx| cx.notify(entity.entity_id()));
        }
    }

    #[doc = "Mutates current state and notifies its readers, returning None after disposal."]
    pub fn update<C: super::SignalContext, R>(
        self,
        cx: &mut C,
        update: impl FnOnce(&mut T) -> R,
    ) -> Option<R> {
        let entity = cx.signal_entity(self)?;
        Some(entity.update(cx, |value, cx| {
            let result = update(value);
            cx.notify();
            result
        }))
    }

    #[doc = "Releases the registry's ownership. Existing handles cannot reach a replacement value."]
    pub fn dispose(self, cx: &mut App) {
        if cx
            .try_global::<Store<T>>()
            .is_some_and(|store| store.identity == self.key.registry)
        {
            cx.global_mut::<Store<T>>().values.remove(self.key.slot);
        }
    }

    #[doc = "Whether this handle still refers to a registered entity in this application."]
    pub fn is_alive(self, cx: &App) -> bool {
        self.resolve(cx).is_some()
    }
}
