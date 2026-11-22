use crate::gpui::{App, AppContext, AsyncApp, Context, Entity};

use super::Signal;

#[doc = "GPUI contexts that can resolve a signal without requiring its registry to exist."]
pub trait SignalContext: AppContext {
    #[doc(hidden)]
    fn signal_entity<T: 'static>(&self, signal: Signal<T>) -> Option<Entity<T>>;
}

impl SignalContext for App {
    fn signal_entity<T: 'static>(&self, signal: Signal<T>) -> Option<Entity<T>> {
        signal.resolve(self).cloned()
    }
}

impl<V: 'static> SignalContext for Context<'_, V> {
    fn signal_entity<T: 'static>(&self, signal: Signal<T>) -> Option<Entity<T>> {
        signal.resolve(self).cloned()
    }
}

impl SignalContext for AsyncApp {
    fn signal_entity<T: 'static>(&self, signal: Signal<T>) -> Option<Entity<T>> {
        self.try_read_global::<super::signal::Store<T>, _>(|_, cx| signal.resolve(cx).cloned())
            .flatten()
    }
}
