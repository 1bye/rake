//! View-owned loading bound to the existing managed-work scheduler.

#[cfg(all(test, feature = "view-test-support", not(target_family = "wasm")))]
#[path = "binding_tests.rs"]
mod tests;

use std::error::Error;
use std::future::Future;
use std::rc::Rc;

use futures::future::LocalBoxFuture;
use gpui::{App, AppContext, Context, Entity, Subscription, Task};

use super::{LoadState, LoadStatus};
use crate::http::{CancellationToken, HttpClient};
use crate::services::Services;
use crate::tasks::{TaskError, TaskHandle, TaskScope, TaskState};

/// Owned services and cancellation for arbitrary asynchronous preparation.
///
/// Async functions execute on the UI loop and need not be `Send`. Move blocking
/// file, CPU or native work to [`Self::background_executor`]. A separately spawned
/// worker may continue after cancellation; its result will still be rejected.
#[derive(Clone)]
pub struct LoadContext {
    services: Services,
    cancellation: CancellationToken,
    background: gpui::BackgroundExecutor,
}

impl LoadContext {
    pub(crate) fn new(
        services: Services,
        cancellation: CancellationToken,
        background: gpui::BackgroundExecutor,
    ) -> Self {
        Self {
            services,
            cancellation,
            background,
        }
    }

    /// Returns the application's shared services without copying their values.
    pub fn services(&self) -> Services {
        self.services.clone()
    }

    /// Returns the configured application HTTP transport.
    pub fn http(&self) -> HttpClient {
        self.services.http()
    }

    /// Returns the signal for this particular loading attempt.
    pub fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    /// Returns GPUI's worker executor for `Send` background work.
    pub fn background_executor(&self) -> gpui::BackgroundExecutor {
        self.background.clone()
    }
}

/// A typed source failure or an error from managed-work admission/cancellation.
#[derive(Debug, thiserror::Error)]
pub enum LoadError<E: Error + 'static> {
    /// This loading attempt was cancelled before publishing its value.
    #[error("loading was cancelled")]
    Cancelled,
    /// The application loader returned an error; the original cause is retained.
    #[error("{0}")]
    Source(#[source] E),
    /// The owner stopped, the task was cancelled, or work could not be admitted.
    #[error("{0}")]
    Task(#[source] TaskError),
}

type Factory<T, E> = Rc<dyn Fn(LoadContext) -> LocalBoxFuture<'static, Result<T, E>>>;

struct Model<T: 'static, E: Error + 'static> {
    load: LoadState<T, LoadError<E>>,
    task: Option<TaskHandle>,
    terminal: Option<Task<()>>,
    released: bool,
}

impl<T, E: Error + 'static> Drop for Model<T, E> {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            let _ = task.cancel();
        }
    }
}

/// One view's automatically observed, retryable asynchronous value.
///
/// Construction starts loading. Changes notify the bound view automatically.
/// Dropping this handle cancels its attempt and releases its subscription. A
/// page keeps the handle while retained in history, so finite work continues
/// while hidden. Results require neither `Clone` nor `Send`.
pub struct Loader<T: 'static, E: Error + 'static> {
    model: Entity<Model<T, E>>,
    scope: Result<TaskScope, TaskError>,
    services: Services,
    factory: Option<Factory<T, E>>,
    _observe: Subscription,
    _release: Subscription,
}

impl<T: 'static, E: Error + 'static> Loader<T, E> {
    pub(crate) fn new<V: 'static, F, Fut>(
        scope: Result<TaskScope, TaskError>,
        services: Services,
        factory: F,
        cx: &mut Context<V>,
    ) -> Self
    where
        F: Fn(LoadContext) -> Fut + 'static,
        Fut: Future<Output = Result<T, E>> + 'static,
    {
        let mut loader = Self::bound(
            scope,
            services,
            Some(Rc::new(move |context| Box::pin(factory(context)))),
            cx,
        );

        loader.retry(cx);

        loader
    }

    fn bound<V: 'static>(
        scope: Result<TaskScope, TaskError>,
        services: Services,
        factory: Option<Factory<T, E>>,
        cx: &mut Context<V>,
    ) -> Self {
        let model = cx.new(|_| Model {
            load: LoadState::new(),
            task: None,
            terminal: None,
            released: false,
        });

        let observe = cx.observe(&model, |_, _, cx| cx.notify());

        let release = cx.on_release({
            let weak = model.downgrade();

            move |_, cx| {
                let _ = weak.update(cx, |model, cx| {
                    model.released = true;

                    if let Some(task) = model.task.take() {
                        let _ = task.cancel();
                    }

                    model.terminal = None;

                    if let LoadStatus::Loading(token) = model.load.state() {
                        let token = *token;

                        model
                            .load
                            .complete(token, Err(LoadError::Task(TaskError::OwnerDisposed)));
                        cx.notify();
                    }
                });
            }
        });

        Self {
            model,
            scope,
            services,
            factory,
            _observe: observe,
            _release: release,
        }
    }

    /// Borrows the current value or typed failure without cloning its payload.
    pub fn state<'a>(&'a self, cx: &'a App) -> &'a LoadStatus<T, LoadError<E>> {
        self.model.read(cx).load.state()
    }

    /// Whether this view is waiting for its current attempt.
    pub fn is_loading(&self, cx: &App) -> bool {
        matches!(self.state(cx), LoadStatus::Loading(_))
    }

    /// Cancels the previous attempt and starts the reusable factory again.
    /// Admission and source errors become [`Self::state`] failures.
    pub fn retry(&mut self, cx: &mut App) {
        let factory = self.factory.clone().expect("public Loader handles always have a retry factory; one-shot handles expose AsyncAction");

        self.start(cx, move |load| factory(load));
    }

    fn start<F, Fut>(&mut self, cx: &mut App, factory: F)
    where
        F: FnOnce(LoadContext) -> Fut + 'static,
        Fut: Future<Output = Result<T, E>> + 'static,
    {
        let token = self.model.update(cx, |model, cx| {
            if let Some(task) = model.task.take() {
                let _ = task.cancel();
            }

            model.terminal = None;
            let token = model.load.begin();

            cx.notify();

            if model.released {
                model
                    .load
                    .complete(token, Err(LoadError::Task(TaskError::OwnerDisposed)));

                None
            } else {
                Some(token)
            }
        });

        let Some(token) = token else { return };

        let services = self.services.clone();
        let background = cx.background_executor().clone();
        let weak = self.model.downgrade();
        let mut context = cx.to_async();

        let admitted = self.scope.clone().and_then(|scope| {
            scope.spawn_with("load", move |run| async move {
                let load = LoadContext::new(services, run.cancellation(), background);
                let result = factory(load).await;

                run.check()?;
                let failure = result.as_ref().err().map(ToString::to_string);

                weak.update(&mut context, |model, cx| {
                    if model
                        .load
                        .complete(token, result.map_err(LoadError::Source))
                    {
                        cx.notify();
                    }
                })
                .map_err(|_| TaskError::OwnerDisposed)?;

                match failure {
                    Some(error) => Err(TaskError::Failed(error)),
                    None => Ok(()),
                }
            })
        });

        match admitted {
            Ok(task) => {
                let observed = task.clone();

                let terminal = cx.spawn({
                    let weak = self.model.downgrade();

                    async move |cx| {
                        let terminal = observed.wait().await;

                        let error = match terminal {
                            Ok(TaskState::Cancelled) => LoadError::Cancelled,
                            Ok(TaskState::Failed(error)) | Err(error) => LoadError::Task(error),
                            Ok(_) => return,
                        };

                        let _ = weak.update(cx, |model, cx| {
                            if model.load.complete(token, Err(error)) {
                                cx.notify();
                            }
                        });
                    }
                });

                self.model.update(cx, |model, _| {
                    model.task = Some(task);
                    model.terminal = Some(terminal);
                });
            }
            Err(error) => self.model.update(cx, |model, cx| {
                if model.load.complete(token, Err(LoadError::Task(error))) {
                    cx.notify();
                }
            }),
        }
    }

    /// Cancels this attempt immediately and exposes cancellation as a typed failure.
    pub fn cancel(&mut self, cx: &mut App) {
        self.model.update(cx, |model, cx| {
            if let Some(task) = model.task.take() {
                let _ = task.cancel();
            }

            model.terminal = None;

            if let LoadStatus::Loading(token) = model.load.state() {
                let token = *token;

                model.load.complete(token, Err(LoadError::Cancelled));
                cx.notify();
            }
        });
    }
}

/// A view-bound, one-shot user action tracked as finite managed work.
///
/// Initiate browser picker/clipboard requests inside the event handler, then pass
/// the returned future to `runtime.action` or `page.action`. Changes redraw the
/// view; dropping/cancelling the handle stops observation and managed work. An
/// external OS action already initiated cannot be undone. There is no automatic
/// retry: a browser operation may require a fresh user gesture.
pub struct AsyncAction<T: 'static, E: Error + 'static> {
    loader: Loader<T, E>,
}

impl<T: 'static, E: Error + 'static> AsyncAction<T, E> {
    pub(crate) fn new<V: 'static, Fut>(
        scope: Result<TaskScope, TaskError>,
        services: Services,
        future: Fut,
        cx: &mut Context<V>,
    ) -> Self
    where
        Fut: Future<Output = Result<T, E>> + 'static,
    {
        let mut loader = Loader::bound(scope, services, None, cx);

        loader.start(cx, move |_| future);

        Self { loader }
    }

    /// Borrows the completed value, typed failure or current loading status.
    pub fn state<'a>(&'a self, cx: &'a App) -> &'a LoadStatus<T, LoadError<E>> {
        self.loader.state(cx)
    }

    /// Whether managed work is still pending.
    pub fn is_running(&self, cx: &App) -> bool {
        self.loader.is_loading(cx)
    }

    /// Cancels observation/work; external effects cannot be rolled back.
    pub fn cancel(&mut self, cx: &mut App) {
        self.loader.cancel(cx);
    }
}
