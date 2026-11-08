use std::any::{Any, type_name};
use std::cell::RefCell;
use std::error::Error;
use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

use crate::gpui::{AnyElement, App, Context, Global, Render, Subscription, Window};
use crate::loading::{LoadContext, Loader};
use crate::router::Route;
use crate::runtime::RuntimeContext;

mod element;
mod handles;
#[cfg(feature = "devtools")]
mod inspection;

#[cfg(all(test, feature = "view-test-support", not(target_family = "wasm")))]
mod tests;

pub use element::ComponentElement;
pub use handles::{ComponentContext, ComponentError, Resource, State, StateRef};

use handles::{Handle, Registration, Registry};

#[doc(hidden)]
pub trait FunctionComponent: Clone + 'static {
    fn component_id(&self) -> crate::gpui::ElementId;
    fn render_component(&self, cx: &mut Scope<'_, '_>) -> AnyElement;
    fn equivalent(&self, _: &Self) -> bool {
        false
    }
}

trait Props: Any {
    fn render(&self, cx: &mut Scope<'_, '_>) -> AnyElement;
    #[cfg(feature = "devtools")]
    fn type_name(&self) -> &'static str;
}

impl<P: FunctionComponent> Props for RefCell<P> {
    fn render(&self, cx: &mut Scope<'_, '_>) -> AnyElement {
        self.borrow().render_component(cx)
    }

    #[cfg(feature = "devtools")]
    fn type_name(&self) -> &'static str {
        type_name::<ComponentElement<P>>()
    }
}

struct Instance {
    props: Rc<dyn Props>,
    hooks: Rc<RefCell<Hooks>>,
    registration: Registration,
}

impl Render for Instance {
    fn render(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl crate::gpui::IntoElement {
        let mut hooks = self.hooks.borrow_mut();
        hooks.cursor = 0;
        let element = self.props.render(&mut Scope {
            hooks: &mut hooks,
            owner: self.registration.handle(),
            window,
            cx,
        });

        assert_eq!(
            hooks.cursor,
            hooks.slots.len(),
            "component hook count changed: call hooks unconditionally in the same order"
        );
        hooks.mounted = true;

        element::ComponentBody {
            child: element,
            owner: cx.weak_entity(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    State,
    Ref,
    Memo,
    Effect,
    Context,
    Resource,
}

type Cleanup = Box<dyn FnOnce()>;
type EffectCallback = Box<dyn FnOnce(&mut App) -> Option<Cleanup>>;

#[derive(Default)]
struct Effect {
    revision: u64,
    pending: Option<EffectCallback>,
    cleanup: Option<Cleanup>,
}

impl Drop for Effect {
    fn drop(&mut self) {
        if let Some(cleanup) = self.cleanup.take() {
            cleanup();
        }
    }
}

struct Slot {
    kind: Kind,
    type_name: &'static str,
    value: Box<dyn Any>,
    effect: Option<Box<Effect>>,
}

#[derive(Default)]
struct Hooks {
    slots: Vec<Slot>,
    cursor: usize,
    mounted: bool,
    effect_queued: bool,
    pending_effects: bool,
}

impl Hooks {
    fn next<T: 'static>(&mut self, kind: Kind, init: impl FnOnce() -> T) -> (usize, &mut T) {
        let index = self.cursor;
        self.cursor += 1;

        if index == self.slots.len() {
            assert!(
                !self.mounted,
                "component hook count increased at slot {index}: call hooks unconditionally in the same order"
            );
            self.slots.push(Slot {
                kind,
                type_name: type_name::<T>(),
                value: Box::new(init()),
                effect: None,
            });
        }

        let slot = &mut self.slots[index];
        let value = slot.value.downcast_mut::<T>();
        assert!(
            slot.kind == kind && value.is_some(),
            "component hook order/type changed at slot {index}: expected {:?}<{}>, received {kind:?}<{}>; call hooks unconditionally in the same order",
            slot.kind,
            slot.type_name,
            type_name::<T>(),
        );

        (index, value.unwrap())
    }
}

#[doc = "A component render's borrowed access to persistent hooks, its window, and GPUI."]
pub struct Scope<'a, 'cx> {
    hooks: &'a mut Hooks,
    owner: Handle,
    window: &'a mut Window,
    cx: &'a mut Context<'cx, Instance>,
}

impl Scope<'_, '_> {
    #[doc = "Reads an owned snapshot and a Copy setter. Initialization runs once per mount."]
    pub fn use_state<T: Clone + 'static>(&mut self, init: impl FnOnce() -> T) -> (T, State<T>) {
        let (index, value) = self.hooks.next(Kind::State, init);

        (value.clone(), State::new(self.owner.slot(index)))
    }

    #[doc = "Retains mutable data whose updates do not notify the component."]
    pub fn use_ref<T: 'static>(&mut self, init: impl FnOnce() -> T) -> StateRef<T> {
        let (index, _) = self.hooks.next(Kind::Ref, init);

        StateRef::new(self.owner.slot(index))
    }

    #[doc = "Clones a computed value, recomputing only when the dependencies compare unequal."]
    pub fn use_memo<D: PartialEq + 'static, T: Clone + 'static>(
        &mut self,
        dependencies: D,
        compute: impl FnOnce(&D) -> T,
    ) -> T {
        let (_, stored) = self.hooks.next::<Option<(D, T)>>(Kind::Memo, || None);

        if stored
            .as_ref()
            .is_none_or(|(previous, _)| previous != &dependencies)
        {
            let value = compute(&dependencies);
            *stored = Some((dependencies, value));
        }

        stored.as_ref().unwrap().1.clone()
    }

    #[doc = "Runs an effect after paint. Return () or a cleanup closure; cleanup runs before replacement and on unmount."]
    pub fn use_effect<D, F, R>(&mut self, dependencies: D, effect: F)
    where
        D: PartialEq + 'static,
        F: FnOnce(&mut App) -> R + 'static,
        R: EffectResult,
    {
        let (index, previous) = self.hooks.next::<Option<D>>(Kind::Effect, || None);

        if previous.as_ref() != Some(&dependencies) {
            *previous = Some(dependencies);
            let state = self.hooks.slots[index]
                .effect
                .get_or_insert_with(Default::default);
            state.revision = state
                .revision
                .checked_add(1)
                .expect("effect revision exhausted");
            state.pending = Some(Box::new(move |cx| effect(cx).into_cleanup()));
            self.hooks.pending_effects = true;
        }
    }

    #[doc = "Reads a GPUI global and subscribes this component to its changes. The global must already be installed."]
    pub fn use_context<G: Global + Clone>(&mut self) -> G {
        self.hooks.next(Kind::Context, || {
            let owner = self.cx.weak_entity();
            ContextSlot::<G> {
                _subscription: App::observe_global::<G>(self.cx, move |cx| {
                    if let Some(owner) = owner.upgrade() {
                        cx.notify(owner.entity_id());
                    }
                }),
                _type: std::marker::PhantomData,
            }
        });

        self.cx.global::<G>().clone()
    }

    #[doc = "Observes a theme installed through rake::theme::install."]
    pub fn use_theme<T: 'static>(&mut self) -> Rc<T> {
        self.use_context::<crate::theme::ThemeGlobal<T>>().0
    }

    #[doc = "Binds a managed loader to this component. Unequal dependencies replace and cancel the previous loader."]
    pub fn use_resource<R, D, T, E, F, Fut>(
        &mut self,
        runtime: &RuntimeContext<R>,
        dependencies: D,
        factory: F,
    ) -> Resource<T, E>
    where
        R: Route,
        D: Clone + PartialEq + 'static,
        T: 'static,
        E: Error + 'static,
        F: Fn(D, LoadContext) -> Fut + 'static,
        Fut: Future<Output = Result<T, E>> + 'static,
    {
        let (index, stored) = self
            .hooks
            .next::<Option<ResourceSlot<T, E>>>(Kind::Resource, || None);
        let changed = stored.as_ref().is_none_or(|previous| {
            previous
                .dependencies
                .downcast_ref::<D>()
                .expect("resource dependency type changed")
                != &dependencies
        });

        if changed {
            drop(stored.take());
            let input = dependencies.clone();
            let loader = runtime.load(self.cx, move |load| factory(input.clone(), load));
            *stored = Some(ResourceSlot {
                dependencies: Box::new(dependencies),
                loader,
            });
        }

        Resource::new(self.owner.slot(index))
    }

    #[doc = "Borrows the component's window for GPUI operations during this render."]
    pub fn window(&mut self) -> &mut Window {
        self.window
    }
}

impl Deref for Scope<'_, '_> {
    type Target = App;

    fn deref(&self) -> &App {
        self.cx
    }
}

impl DerefMut for Scope<'_, '_> {
    fn deref_mut(&mut self) -> &mut App {
        self.cx
    }
}

#[doc = "The supported effect results: no cleanup, or an owned cleanup closure."]
pub trait EffectResult: 'static {
    #[doc(hidden)]
    fn into_cleanup(self) -> Option<Box<dyn FnOnce()>>;
}

impl EffectResult for () {
    fn into_cleanup(self) -> Option<Cleanup> {
        None
    }
}

impl<F: FnOnce() + 'static> EffectResult for F {
    fn into_cleanup(self) -> Option<Cleanup> {
        Some(Box::new(self))
    }
}

struct ContextSlot<G> {
    _subscription: Subscription,
    _type: std::marker::PhantomData<G>,
}

struct ResourceSlot<T: 'static, E: Error + 'static> {
    dependencies: Box<dyn Any>,
    loader: Loader<T, E>,
}

fn flush_effects(owner: crate::gpui::WeakEntity<Instance>, cx: &mut App) {
    let Some(instance) = owner.upgrade() else {
        return;
    };
    let hooks = instance.read(cx).hooks.clone();
    drop(instance);
    let pending: Vec<_> = {
        let mut hooks = hooks.borrow_mut();
        hooks.effect_queued = false;
        hooks.pending_effects = false;
        hooks
            .slots
            .iter_mut()
            .enumerate()
            .filter_map(|(index, slot)| {
                let effect = slot.effect.as_mut()?;
                Some((
                    index,
                    effect.revision,
                    effect.pending.take()?,
                    effect.cleanup.take(),
                ))
            })
            .collect()
    };

    for (index, revision, effect, cleanup) in pending {
        if let Some(cleanup) = cleanup {
            cleanup();
        }

        if owner.upgrade().is_none() {
            continue;
        }

        let mut cleanup = effect(cx);
        if owner.upgrade().is_some()
            && let Some(effect) = hooks
                .borrow_mut()
                .slots
                .get_mut(index)
                .and_then(|slot| slot.effect.as_mut())
            && effect.revision == revision
        {
            effect.cleanup = cleanup.take();
        }

        if let Some(cleanup) = cleanup {
            cleanup();
        }
    }
}
