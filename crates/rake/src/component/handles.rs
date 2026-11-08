use std::cell::{Cell, RefCell};
use std::error::Error;
use std::marker::PhantomData;
use std::rc::Rc;

use slotmap::{DefaultKey, SlotMap};

use crate::gpui::{App, AsyncApp, Context, Entity, Global, WeakEntity};
use crate::loading::{LoadError, LoadStatus, Loader};

use super::{Hooks, Instance, ResourceSlot, Scope};

thread_local! {
    static NEXT_REGISTRY: Cell<u64> = const { Cell::new(0) };
}

#[derive(Clone)]
pub(super) struct Registry(Rc<RefCell<Arena>>);

struct Arena {
    id: u64,
    entries: SlotMap<DefaultKey, WeakEntity<Instance>>,
}

impl Global for Registry {}

impl Default for Registry {
    fn default() -> Self {
        let id = NEXT_REGISTRY.with(|next| {
            let id = next
                .get()
                .checked_add(1)
                .expect("component registry identity exhausted");
            next.set(id);
            id
        });

        Self(Rc::new(RefCell::new(Arena {
            id,
            entries: SlotMap::with_key(),
        })))
    }
}

impl Registry {
    pub(super) fn register(&self, owner: WeakEntity<Instance>) -> Registration {
        let mut arena = self.0.borrow_mut();
        let key = arena.entries.insert(owner);

        Registration {
            registry: self.clone(),
            handle: Handle {
                registry: arena.id,
                key,
                slot: 0,
            },
        }
    }
}

pub(super) struct Registration {
    registry: Registry,
    handle: Handle,
}

impl Registration {
    pub(super) fn handle(&self) -> Handle {
        self.handle
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.registry.0.borrow_mut().entries.remove(self.handle.key);
    }
}

#[doc(hidden)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Handle {
    registry: u64,
    key: DefaultKey,
    slot: u32,
}

impl Handle {
    pub(super) fn slot(self, index: usize) -> Self {
        Self {
            slot: u32::try_from(index).expect("component hook capacity exhausted"),
            ..self
        }
    }

    fn owner(&self, cx: &App) -> Result<Entity<Instance>, ComponentError> {
        let registry = cx
            .try_global::<Registry>()
            .ok_or(ComponentError::Disposed)?;
        let arena = registry.0.borrow();
        if arena.id != self.registry {
            return Err(ComponentError::Disposed);
        }

        arena
            .entries
            .get(self.key)
            .and_then(WeakEntity::upgrade)
            .ok_or(ComponentError::Disposed)
    }

    fn update<T: 'static, R>(
        self,
        cx: &mut App,
        notify: bool,
        update: impl FnOnce(&mut T, &mut App) -> R,
    ) -> Result<R, ComponentError> {
        let owner = self.owner(cx)?;
        owner.update(cx, |instance, cx| {
            instance.hooks.update(self.slot, cx, notify, update)
        })
    }
}

impl Hooks {
    fn update<T: 'static, R>(
        &mut self,
        slot: u32,
        cx: &mut Context<Instance>,
        notify: bool,
        update: impl FnOnce(&mut T, &mut App) -> R,
    ) -> Result<R, ComponentError> {
        let value = self
            .slots
            .get_mut(slot as usize)
            .ok_or(ComponentError::Disposed)?
            .value
            .downcast_mut::<T>()
            .ok_or(ComponentError::TypeChanged)?;
        let result = update(value, cx);

        if notify {
            cx.notify();
        }

        Ok(result)
    }
}

#[doc = "A hook handle cannot access its former component or its expected slot type."]
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ComponentError {
    #[doc = "The component was unmounted, replaced, or belongs to another application."]
    #[error("component has been disposed")]
    Disposed,
    #[doc = "The hook's type changed without a fresh component mount."]
    #[error("component hook type changed")]
    TypeChanged,
}

#[doc = "GPUI contexts accepted by component handles on the UI thread."]
pub trait ComponentContext {
    #[doc(hidden)]
    fn with_component_app<R>(&mut self, update: impl FnOnce(&mut App) -> R) -> R;

    #[doc(hidden)]
    fn with_component_slot<T: 'static, R>(
        &mut self,
        handle: Handle,
        notify: bool,
        update: impl FnOnce(&mut T, &mut App) -> R,
    ) -> Result<R, ComponentError> {
        self.with_component_app(|cx| handle.update(cx, notify, update))
    }
}

impl ComponentContext for App {
    fn with_component_app<R>(&mut self, update: impl FnOnce(&mut App) -> R) -> R {
        update(self)
    }
}

impl<T> ComponentContext for Context<'_, T> {
    fn with_component_app<R>(&mut self, update: impl FnOnce(&mut App) -> R) -> R {
        update(self)
    }
}

impl ComponentContext for AsyncApp {
    fn with_component_app<R>(&mut self, update: impl FnOnce(&mut App) -> R) -> R {
        self.update(update)
    }
}

impl ComponentContext for Scope<'_, '_> {
    fn with_component_app<R>(&mut self, update: impl FnOnce(&mut App) -> R) -> R {
        update(self)
    }

    fn with_component_slot<T: 'static, R>(
        &mut self,
        handle: Handle,
        notify: bool,
        update: impl FnOnce(&mut T, &mut App) -> R,
    ) -> Result<R, ComponentError> {
        if handle.slot(0) != self.owner {
            return self.with_component_app(|cx| handle.update(cx, notify, update));
        }

        self.hooks.update(handle.slot, self.cx, notify, update)
    }
}

#[doc(hidden)]
pub struct Hook<T: 'static, const NOTIFY: bool> {
    handle: Handle,
    value: PhantomData<fn() -> T>,
    thread: PhantomData<Rc<()>>,
}

#[doc = "A Copy handle that updates one component's state and notifies its entity."]
pub type State<T> = Hook<T, true>;

#[doc = "A Copy handle to component-owned data; mutations do not trigger a redraw."]
pub type StateRef<T> = Hook<T, false>;

impl<T, const NOTIFY: bool> Copy for Hook<T, NOTIFY> {}

impl<T, const NOTIFY: bool> Clone for Hook<T, NOTIFY> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T, const NOTIFY: bool> PartialEq for Hook<T, NOTIFY> {
    fn eq(&self, other: &Self) -> bool {
        self.handle == other.handle
    }
}

impl<T, const NOTIFY: bool> Eq for Hook<T, NOTIFY> {}

impl<T, const NOTIFY: bool> Hook<T, NOTIFY> {
    pub(super) fn new(handle: Handle) -> Self {
        Self {
            handle,
            value: PhantomData,
            thread: PhantomData,
        }
    }

    #[doc = "Borrows the current value for one callback, without cloning it."]
    pub fn read<R>(
        self,
        cx: &mut impl ComponentContext,
        read: impl FnOnce(&T) -> R,
    ) -> Result<R, ComponentError> {
        cx.with_component_slot(self.handle, false, |value: &mut T, _| read(value))
    }

    #[doc = "Updates the latest stored value. State notifies its owner; refs do not."]
    pub fn update<R>(
        self,
        cx: &mut impl ComponentContext,
        update: impl FnOnce(&mut T) -> R,
    ) -> Result<R, ComponentError> {
        cx.with_component_slot(self.handle, NOTIFY, |value, _| update(value))
    }
}

impl<T> State<T> {
    #[doc = "Replaces the value and notifies its component; returns Disposed after unmount."]
    pub fn set(self, cx: &mut impl ComponentContext, value: T) -> Result<(), ComponentError> {
        self.update(cx, |state| *state = value)
    }
}

#[doc = "A Copy handle to a managed loader owned by one component hook."]
pub struct Resource<T: 'static, E: Error + 'static>(StateRef<Option<ResourceSlot<T, E>>>);

impl<T, E: Error> Copy for Resource<T, E> {}

impl<T, E: Error> Clone for Resource<T, E> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T, E: Error> Resource<T, E> {
    pub(super) fn new(handle: Handle) -> Self {
        Self(StateRef::new(handle))
    }

    fn with_loader<R>(
        self,
        cx: &mut impl ComponentContext,
        update: impl FnOnce(&mut Loader<T, E>, &mut App) -> R,
    ) -> Result<R, ComponentError> {
        cx.with_component_slot(
            self.0.handle,
            false,
            |slot: &mut Option<ResourceSlot<T, E>>, cx| {
                update(&mut slot.as_mut().unwrap().loader, cx)
            },
        )
    }

    #[doc = "Borrows the current loading state without requiring Clone on the result or error."]
    pub fn read<R>(
        self,
        cx: &mut impl ComponentContext,
        read: impl FnOnce(&LoadStatus<T, LoadError<E>>) -> R,
    ) -> Result<R, ComponentError> {
        self.with_loader(cx, |loader, cx| read(loader.state(cx)))
    }

    #[doc = "Cancels the previous attempt and retries the current resource factory."]
    pub fn retry(self, cx: &mut impl ComponentContext) -> Result<(), ComponentError> {
        self.with_loader(cx, |loader, cx| loader.retry(cx))
    }

    #[doc = "Cancels the current attempt and exposes the loader's cancelled state."]
    pub fn cancel(self, cx: &mut impl ComponentContext) -> Result<(), ComponentError> {
        self.with_loader(cx, |loader, cx| loader.cancel(cx))
    }
}
