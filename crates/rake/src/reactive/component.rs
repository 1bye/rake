use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

use crate::gpui::{
    AnyElement, AnyEntity, App, AsyncApp, Context, EntityId, Global, Render, Task, Window,
};

use super::signal::{Key, Signal};

struct Registration {
    key: Key,
    dispose: fn(Key, &mut App),
}

struct Retained {
    _entity: AnyEntity,
    #[cfg(feature = "view-test-support")]
    _lifetime: super::stats::Lifetime,
}

#[derive(Default)]
pub(super) struct Owner {
    signals: Vec<Registration>,
    #[cfg(feature = "devtools")]
    inspection: Vec<super::inspection::Entry>,
    tasks: Vec<Task<()>>,
    retained: Vec<Retained>,
    cleanups: Vec<super::effect::Cleanup>,
}

impl Owner {
    fn dispose(&mut self, cx: &mut App) {
        self.tasks.clear();
        for cleanup in self.cleanups.drain(..).rev() {
            cleanup(cx);
        }
        self.retained.clear();
        for signal in self.signals.drain(..) {
            (signal.dispose)(signal.key, cx);
        }
    }
}

struct Provider {
    parent: EntityId,
    values: HashMap<TypeId, Rc<dyn Any>>,
}

#[derive(Default)]
struct Providers(HashMap<EntityId, Provider>);
impl Global for Providers {}

#[doc = "Mount-time access to the component's real GPUI context and owned work."]
pub struct Setup<'a, 'cx, V: 'static> {
    cx: &'a mut Context<'cx, V>,
    window: &'a mut Window,
    owner: Owner,
    finished: bool,
}

impl<'a, 'cx, V: 'static> Setup<'a, 'cx, V> {
    #[doc(hidden)]
    pub fn new(window: &'a mut Window, cx: &'a mut Context<'cx, V>) -> Self {
        let id = cx.entity_id();
        let parent = window.current_view();
        cx.default_global::<Providers>().0.insert(
            id,
            Provider {
                parent,
                values: HashMap::new(),
            },
        );
        Self {
            cx,
            window,
            owner: Owner::default(),
            finished: false,
        }
    }

    #[doc = "Creates a signal that is disposed when this component unmounts."]
    pub fn signal<T: 'static>(&mut self, value: T) -> Signal<T> {
        let signal = Signal::new(self.cx, value);
        self.owner.signals.push(Registration {
            key: signal.key,
            dispose: |key, cx| Signal::<T>::from_key(key).dispose(cx),
        });
        signal
    }

    #[doc(hidden)]
    pub fn inspect_signal<T: 'static>(&mut self, name: &'static str, signal: Signal<T>) {
        self.inspect_binding(name, "signal", signal);
    }

    #[doc(hidden)]
    pub fn inspect_memo<T: 'static>(&mut self, name: &'static str, memo: super::Memo<T>) {
        self.inspect_binding(name, "memo", memo.0);
    }

    #[doc(hidden)]
    pub fn inspect_resource<T: 'static, E: std::error::Error + 'static>(
        &mut self,
        name: &'static str,
        resource: super::Resource<T, E>,
    ) {
        self.inspect_binding(name, "resource", resource.state);
    }

    fn inspect_binding<T: 'static>(
        &mut self,
        name: &'static str,
        kind: &'static str,
        signal: Signal<T>,
    ) {
        #[cfg(feature = "devtools")]
        self.owner
            .inspection
            .push(super::inspection::Entry::new(name, kind, signal));
        #[cfg(not(feature = "devtools"))]
        let _ = (name, kind, signal);
    }

    #[doc = "Owns a foreground task until it completes or this component unmounts."]
    pub fn spawn<R: 'static>(&mut self, task: impl AsyncFnOnce(&mut AsyncApp) -> R + 'static) {
        #[cfg(feature = "view-test-support")]
        let lifetime = super::stats::Lifetime::new(self.cx, super::stats::Kind::Task);
        self.owner.tasks.push(App::spawn(self.cx, async move |cx| {
            #[cfg(feature = "view-test-support")]
            let _lifetime = lifetime;
            task(cx).await;
        }));
    }

    #[doc = "Runs a cleanup while the application is still available at unmount."]
    pub fn on_cleanup(&mut self, cleanup: impl FnOnce(&mut App) + 'static) {
        self.owner.cleanups.push(Box::new(cleanup));
    }

    #[doc = "Provides a value to descendant function components. Provide a signal for reactive updates."]
    pub fn provide<T: Clone + 'static>(&mut self, value: T) {
        let id = self.cx.entity_id();
        self.cx
            .global_mut::<Providers>()
            .0
            .get_mut(&id)
            .unwrap()
            .values
            .insert(TypeId::of::<T>(), Rc::new(value));
    }

    #[doc = "Looks up the closest provider once during setup. Ordinary GPUI views interrupt the provider chain."]
    pub fn context<T: Clone + 'static>(&self) -> Option<T> {
        let providers = self.cx.global::<Providers>();
        let mut id = self.cx.entity_id();
        while let Some(provider) = providers.0.get(&id) {
            if let Some(value) = provider.values.get(&TypeId::of::<T>()) {
                return value.downcast_ref::<T>().cloned();
            }
            if id == provider.parent {
                break;
            }
            id = provider.parent;
        }
        None
    }

    #[doc = "Borrows the real window during setup."]
    pub fn window(&mut self) -> &mut Window {
        self.window
    }

    pub(super) fn retain<T: 'static>(&mut self, entity: crate::gpui::Entity<T>) {
        self.owner.retained.push(Retained {
            _entity: entity.into_any(),
            #[cfg(feature = "view-test-support")]
            _lifetime: super::stats::Lifetime::new(self.cx, super::stats::Kind::Observer),
        });
    }

    #[doc(hidden)]
    pub fn finish<P: 'static, F>(self, props: P, render: F) -> ComponentView<P, F>
    where
        F: FnMut(&P, &mut Window, &mut App) -> AnyElement + 'static,
    {
        self.finish_with(props, render, |_, _| {})
    }
    #[doc(hidden)]
    pub fn finish_with<P: 'static, F, U>(
        mut self,
        props: P,
        render: F,
        update: U,
    ) -> ComponentView<P, F, U>
    where
        F: FnMut(&P, &mut Window, &mut App) -> AnyElement + 'static,
        U: Fn(&P, &mut App) + 'static,
    {
        self.finished = true;
        ComponentView {
            props,
            render,
            update,
            owner: std::mem::take(&mut self.owner),
            subscriptions: super::tracking::Subscriptions::default(),
            #[cfg(feature = "view-test-support")]
            _lifetime: super::stats::Lifetime::new(self.cx, super::stats::Kind::Component),
        }
    }
}

impl<V: 'static> Drop for Setup<'_, '_, V> {
    fn drop(&mut self) {
        if !self.finished {
            self.owner.dispose(self.cx);
            let id = self.cx.entity_id();
            self.cx.global_mut::<Providers>().0.remove(&id);
        }
    }
}

impl<'cx, V: 'static> Deref for Setup<'_, 'cx, V> {
    type Target = Context<'cx, V>;
    fn deref(&self) -> &Self::Target {
        self.cx
    }
}

impl<V: 'static> DerefMut for Setup<'_, '_, V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.cx
    }
}

#[doc(hidden)]
pub struct ComponentView<P, F, U = fn(&P, &mut App)> {
    props: P,
    render: F,
    update: U,
    owner: Owner,
    subscriptions: super::tracking::Subscriptions,
    #[cfg(feature = "view-test-support")]
    _lifetime: super::stats::Lifetime,
}

impl<P: 'static, F: 'static, U: Fn(&P, &mut App) + 'static> ComponentView<P, F, U> {
    #[doc(hidden)]
    pub fn observe_release(cx: &Context<Self>) {
        let id = cx.entity_id();
        cx.on_release(move |this, cx| {
            this.owner.dispose(cx);
            cx.global_mut::<Providers>().0.remove(&id);
        })
        .detach();
    }

    #[doc(hidden)]
    pub fn swap_props(
        &mut self,
        props: P,
        cx: &mut Context<Self>,
        equivalent: impl FnOnce(&P, &P) -> bool,
    ) -> bool {
        if equivalent(&self.props, &props) {
            return false;
        }
        (self.update)(&props, cx);
        self.props = props;
        true
    }
}

impl<P: crate::gpui::RenderOnce, F, U: 'static> Render for ComponentView<P, F, U>
where
    F: FnMut(&P, &mut Window, &mut App) -> AnyElement + 'static,
{
    fn render(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl crate::gpui::IntoElement {
        let (child, dependencies) =
            super::tracking::collect(cx, |cx| (self.render)(&self.props, window, cx));
        super::element::Rendered {
            owner: cx.weak_entity(),
            child,
            dependencies,
        }
    }
}

pub(super) trait ReactiveView: 'static {
    #[cfg(feature = "devtools")]
    fn inspect(&self, cx: &mut App);
    fn update_dependencies(
        &mut self,
        dependencies: Vec<super::tracking::Dependency>,
        target: crate::gpui::AnyWeakEntity,
        cx: &mut App,
    );
}

impl<P: crate::gpui::RenderOnce, F: 'static, U: 'static> ReactiveView for ComponentView<P, F, U> {
    #[cfg(feature = "devtools")]
    fn inspect(&self, cx: &mut App) {
        let (signals, truncated) = super::inspection::capture(&self.owner.inspection, cx);
        crate::inspection::component_signals(
            cx,
            std::any::type_name::<crate::gpui::ViewElement<P>>(),
            signals,
            truncated,
        );
    }
    fn update_dependencies(
        &mut self,
        dependencies: Vec<super::tracking::Dependency>,
        target: crate::gpui::AnyWeakEntity,
        cx: &mut App,
    ) {
        self.subscriptions.replace(dependencies, target, cx);
    }
}
