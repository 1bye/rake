use std::cell::RefCell;
use std::rc::Rc;

use crate::gpui::{AnyWindowHandle, App, AppContext, Context, Global, Window};

use super::{Setup, tracking};

pub(super) type Cleanup = Box<dyn FnOnce(&mut App)>;

#[derive(Clone, Default)]
struct Cleanups(Rc<RefCell<Vec<Vec<Cleanup>>>>);
impl Global for Cleanups {}

struct CleanupScope(Cleanups);
impl Drop for CleanupScope {
    fn drop(&mut self) {
        self.0.0.borrow_mut().pop();
    }
}

#[doc = "Additional application operations used inside reactive callbacks."]
pub trait ReactiveApp {
    #[doc = "Reads a signal installed with provide_global. Panics when it is missing."]
    fn global_signal<T: 'static>(&self) -> super::Signal<T>;
    #[doc = "Registers cleanup for the running effect, before its next execution or disposal."]
    fn on_cleanup(&mut self, cleanup: impl FnOnce(&mut App) + 'static);
}

impl ReactiveApp for App {
    fn global_signal<T: 'static>(&self) -> super::Signal<T> {
        self.global::<super::global::GlobalSignal<T>>().0
    }

    fn on_cleanup(&mut self, cleanup: impl FnOnce(&mut App) + 'static) {
        self.default_global::<Cleanups>()
            .0
            .borrow_mut()
            .last_mut()
            .expect("on_cleanup requires a running effect")
            .push(Box::new(cleanup));
    }
}

struct Effect<F> {
    callback: F,
    window: AnyWindowHandle,
    queued: bool,
    cleanups: Vec<Cleanup>,
    subscriptions: tracking::Subscriptions,
}

impl<F: FnMut(&mut Window, &mut App) + 'static> Effect<F> {
    fn schedule(&mut self, cx: &mut Context<Self>) {
        if self.queued {
            return;
        }
        self.queued = true;
        let owner = cx.weak_entity();
        let window = self.window;
        cx.defer(move |cx| {
            let _ = cx.update_window(window, |_, window, _| {
                window.on_next_frame(move |window, cx| {
                    let _ = owner.update(cx, |effect, cx| effect.run(window, cx));
                });
            });
        });
    }

    fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.queued = false;
        for cleanup in self.cleanups.drain(..).rev() {
            cleanup(cx);
        }
        let cleanups = cx.default_global::<Cleanups>().clone();
        cleanups.0.borrow_mut().push(Vec::new());
        let scope = CleanupScope(cleanups);
        let (_, dependencies) = tracking::collect(cx, |cx| (self.callback)(window, cx));
        self.cleanups = std::mem::take(scope.0.0.borrow_mut().last_mut().unwrap());
        self.subscriptions
            .replace(dependencies, cx.entity().into_any().downgrade(), cx);
    }
}

impl<V: 'static> Setup<'_, '_, V> {
    #[doc = "Runs after the next frame and again after a tracked signal changes."]
    pub fn effect(&mut self, callback: impl FnMut(&mut Window, &mut App) + 'static) {
        let window = self.window().window_handle();
        let effect = App::new(self, |cx| {
            cx.observe_self(|this: &mut Effect<_>, cx| this.schedule(cx))
                .detach();
            cx.on_release(|effect: &mut Effect<_>, cx| {
                for cleanup in effect.cleanups.drain(..).rev() {
                    cleanup(cx);
                }
            })
            .detach();
            Effect {
                callback,
                window,
                queued: true,
                cleanups: Vec::new(),
                subscriptions: tracking::Subscriptions::default(),
            }
        });
        let weak = effect.downgrade();
        self.window().on_next_frame(move |window, cx| {
            let _ = weak.update(cx, |effect, cx| effect.run(window, cx));
        });
        let cleanup_owner = effect.downgrade();
        self.on_cleanup(move |cx| {
            let _ = cleanup_owner.update(cx, |effect, cx| {
                for cleanup in effect.cleanups.drain(..).rev() {
                    cleanup(cx);
                }
            });
        });
        self.retain(effect);
    }
}
