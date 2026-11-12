use std::error::Error;
use std::future::Future;

use crate::gpui::{App, Task};
use crate::loading::{LoadState, LoadStatus};

use super::{Setup, Signal, tracking};

pub(super) struct State<T, E> {
    load: LoadState<T, E>,
    latest: Option<T>,
    task: Option<Task<()>>,
}

#[doc = "A component-owned, reactive request with cancellation, retry, and the last successful value."]
pub struct Resource<T: 'static, E: Error + 'static> {
    pub(super) state: Signal<State<T, E>>,
    retry: Signal<u64>,
}

impl<T, E: Error> Copy for Resource<T, E> {}
impl<T, E: Error> Clone for Resource<T, E> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static, E: Error + 'static> Resource<T, E> {
    #[doc = "Borrows and tracks the current loading status."]
    pub fn read(self, cx: &App) -> &LoadStatus<T, E> {
        self.state.read(cx).load.state()
    }

    #[doc = "Returns the last successful value, including while a replacement request is loading."]
    pub fn latest(self, cx: &App) -> Option<&T> {
        let state = self.state.read(cx);
        match state.load.state() {
            LoadStatus::Ready(value) => Some(value),
            _ => state.latest.as_ref(),
        }
    }

    #[doc = "Whether the current request is in flight."]
    pub fn is_loading(self, cx: &App) -> bool {
        matches!(self.read(cx), LoadStatus::Loading(_))
    }

    #[doc = "Requests a new attempt, cancelling an existing attempt when the observer runs."]
    pub fn retry<C: super::SignalContext>(self, cx: &mut C) {
        self.retry
            .update(cx, |version| *version = version.wrapping_add(1));
    }

    fn start(self, cx: &mut App, future: impl Future<Output = Result<T, E>> + 'static) {
        let Some(entity) = self.state.resolve(cx).cloned() else {
            return;
        };
        entity.update(cx, |state, cx| {
            state.task.take();
            if let Some(value) = state.load.take_ready() {
                state.latest = Some(value);
            }
            let token = state.load.begin();
            let weak = cx.weak_entity();
            #[cfg(feature = "view-test-support")]
            let lifetime = super::stats::Lifetime::new(cx, super::stats::Kind::Task);
            state.task = Some(cx.spawn(async move |_, cx| {
                #[cfg(feature = "view-test-support")]
                let _lifetime = lifetime;
                let result = future.await;
                let _ = weak.update(cx, |state, cx| {
                    if state.load.complete(token, result) {
                        cx.notify();
                    }
                });
            }));
            cx.notify();
        });
    }
}

impl<V: 'static> Setup<'_, '_, V> {
    #[doc = "Starts a request and refetches when signals read before returning its future change."]
    pub fn resource<T, E, F>(&mut self, factory: impl Fn(&App) -> F + 'static) -> Resource<T, E>
    where
        T: 'static,
        E: Error + 'static,
        F: Future<Output = Result<T, E>> + 'static,
    {
        let resource = Resource {
            state: self.signal(State {
                load: LoadState::new(),
                latest: None,
                task: None,
            }),
            retry: self.signal(0),
        };
        let (future, dependencies) = tracking::collect(self, |cx| {
            resource.retry.get(cx);
            factory(cx)
        });
        resource.start(self, future);
        let observer = tracking::Observer::new(
            self,
            move |cx| {
                resource.retry.get(cx);
                let future = factory(cx);
                resource.start(cx, future);
            },
            dependencies,
        );
        self.retain(observer);
        resource
    }
}
