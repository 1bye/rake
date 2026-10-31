//! GPUI view binding for the application's shared update controller.
//!
//! The binding owns only view-facing state. Provider work and guarded restart
//! authority remain in [`rake_updates::UpdateController`] and the runtime host.

#![cfg(all(feature = "gpui", feature = "updates"))]

use crate::gpui::{AppContext, Context, Entity, Subscription, Task, WeakEntity};
use crate::quit::{QuitOptions, QuitResult};
use crate::router::{Operation, PageContext, Route};
use crate::runtime::RuntimeContext;
use crate::tasks::{ActivityRun, TaskError, TaskHandle};
use futures::future::{Either, LocalBoxFuture, select};
use rake_updates::{Error, UpdateController, UpdateInfo, UpdateSnapshot, UpdateState};
use std::rc::Rc;

type Action = Rc<dyn Fn() -> Result<(), Error>>;
type RestartAction = Rc<dyn Fn() -> Result<Operation<QuitResult>, Error>>;
type RuntimeLive = Rc<dyn Fn() -> bool>;
type RuntimeStopped = Rc<dyn Fn() -> LocalBoxFuture<'static, ()>>;

/// State exposed by an [`UpdateHandle`] to a view.
#[derive(Clone, Debug, PartialEq)]
pub enum UpdateHandleState {
    /// No controller was installed, or the runtime could not resolve one.
    Unavailable,
    /// The handle is observing the shared controller.
    Observed(UpdateState),
}

enum Observation {
    /// Standalone views use a GPUI-owned task. Runtime shutdown is selected
    /// explicitly, so this observer never enters finite-work quit accounting.
    Standalone(Task<()>),
    /// Pages use a visibility-aware entry activity.
    Page(TaskHandle),
}

/// A view-owned projection of the shared update controller.
///
/// The handle is deliberately not `Clone`: each view owns one observation and
/// its GPUI subscriptions. It still shares the provider controller with the
/// application and never starts a second provider driver.
pub struct UpdateHandle {
    model: Entity<UpdateModel>,
    _observe: Subscription,
    _release: Subscription,
}

struct UpdateModel {
    controller: Option<Rc<UpdateController>>,
    snapshot: Option<UpdateSnapshot>,
    last_error: Option<String>,
    check: Action,
    download: Action,
    restart: RestartAction,
    runtime_live: RuntimeLive,
    runtime_stopped: RuntimeStopped,
    observation: Option<Observation>,
    released: bool,
}

impl Drop for UpdateModel {
    fn drop(&mut self) {
        if let Some(observation) = self.observation.take() {
            observation.cancel();
        }
    }
}

/// Binds updates to a standalone view without making availability fallible.
pub(crate) fn bind_updates<R: Route, V: 'static>(
    runtime: RuntimeContext<R>,
    cx: &mut Context<V>,
) -> UpdateHandle {
    UpdateHandle::new(runtime, cx)
}

/// Binds updates to a retained page using its `while_visible` lifecycle.
pub(crate) fn bind_page_updates<R: Route, V: 'static>(
    page: PageContext<R>,
    cx: &mut Context<V>,
) -> UpdateHandle {
    UpdateHandle::new_page(page, cx)
}

impl UpdateModel {
    fn state(&self) -> UpdateHandleState {
        if self.released || !self.runtime_live() {
            return UpdateHandleState::Unavailable;
        }
        self.snapshot
            .as_ref()
            .map_or(UpdateHandleState::Unavailable, |snapshot| {
                UpdateHandleState::Observed(snapshot.state.clone())
            })
    }

    fn available_version(&self) -> Option<String> {
        if self.released || !self.runtime_live() {
            return None;
        }
        let state = &self.snapshot.as_ref()?.state;
        update_info(state).map(|info| info.version.clone())
    }

    fn can_check(&self) -> bool {
        self.runtime_live()
            && self.controller.is_some()
            && matches!(
                self.snapshot.as_ref().map(|snapshot| &snapshot.state),
                Some(
                    UpdateState::Idle
                        | UpdateState::UpToDate
                        | UpdateState::Available(_)
                        | UpdateState::Failed { .. }
                )
            )
    }

    fn can_download(&self) -> bool {
        self.runtime_live()
            && self.controller.as_ref().is_some_and(|controller| {
                controller.can_install()
                    && matches!(
                        self.snapshot.as_ref().map(|snapshot| &snapshot.state),
                        Some(UpdateState::Available(_))
                    )
            })
    }

    fn can_restart(&self) -> bool {
        self.runtime_live()
            && self.controller.is_some()
            && matches!(
                self.snapshot.as_ref().map(|snapshot| &snapshot.state),
                Some(UpdateState::ReadyToRestart(_))
            )
    }

    fn can_cancel(&self) -> bool {
        self.runtime_live()
            && self.controller.is_some()
            && matches!(
                self.snapshot.as_ref().map(|snapshot| &snapshot.state),
                Some(UpdateState::Checking | UpdateState::Downloading { .. })
            )
    }

    fn runtime_live(&self) -> bool {
        (self.runtime_live)()
    }
}

impl UpdateHandle {
    /// Binds one standalone view to the runtime's shared update controller.
    ///
    /// Binding is intentionally infallible: a missing controller is represented
    /// by [`UpdateHandleState::Unavailable`], while runtime lookup failures are
    /// retained in [`Self::error`].
    pub(crate) fn new<R: Route, V: 'static>(
        runtime: RuntimeContext<R>,
        cx: &mut Context<V>,
    ) -> Self {
        let parts = BindingParts::new(&runtime);
        let model = new_model(parts, cx);
        let (observe, release) = subscriptions(&model, cx);

        let initial_revision = model
            .read(&*cx)
            .snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.revision);
        let controller = model.read(&*cx).controller.clone();
        if let Some(controller) = controller {
            let weak_model = model.downgrade();
            let runtime_stopped = model.read(&*cx).runtime_stopped.clone();
            let observer = cx.spawn(async move |_, cx| {
                observe_managed(
                    controller,
                    weak_model,
                    runtime_stopped,
                    initial_revision,
                    cx,
                )
                .await;
            });
            model.update(cx, |model, _| {
                model.observation = Some(Observation::Standalone(observer));
            });
        }

        Self {
            model,
            _observe: observe,
            _release: release,
        }
    }

    /// Binds one retained page to updates using `PageContext::while_visible`.
    pub(crate) fn new_page<R: Route, V: 'static>(
        page: PageContext<R>,
        cx: &mut Context<V>,
    ) -> Self {
        let runtime = page.runtime();
        let mut parts = BindingParts::new(&runtime);
        let live = parts.runtime_live;
        let entry = page.entry();
        parts.runtime_live = Rc::new(move || live() && entry.id().is_ok());
        let model = new_model(parts, cx);
        let (observe, release) = subscriptions(&model, cx);

        let controller = model.read(&*cx).controller.clone();
        if let Some(controller) = controller {
            let weak_model = model.downgrade();
            let async_cx = cx.to_async();
            let key = format!("rake.updates.{}", model.entity_id());
            let admission = page.while_visible(key, move |run| {
                let controller = controller.clone();
                let weak_model = weak_model.clone();
                let mut async_cx = async_cx.clone();
                async move { observe_page(controller, weak_model, run, &mut async_cx).await }
            });
            match admission {
                Ok(task) => {
                    model.update(cx, |model, _| {
                        model.observation = Some(Observation::Page(task))
                    });
                }
                Err(error) => record_binding_error(&model, cx, error),
            }
        }

        Self {
            model,
            _observe: observe,
            _release: release,
        }
    }

    /// Reads the current view-facing state without provider I/O.
    pub fn state<C: AppContext>(&self, cx: &C) -> UpdateHandleState {
        self.model.read_with(cx, |model, _| model.state())
    }

    /// Returns the currently selected release version, if one exists.
    pub fn available_version<C: AppContext>(&self, cx: &C) -> Option<String> {
        self.model
            .read_with(cx, |model, _| model.available_version())
    }

    /// Returns the most recent local action, binding or runtime error.
    pub fn error<C: AppContext>(&self, cx: &C) -> Option<String> {
        self.model.read_with(cx, |model, _| {
            model.last_error.clone().or_else(|| {
                model
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| match &snapshot.state {
                        UpdateState::Failed { message } => Some(message.clone()),
                        _ => None,
                    })
            })
        })
    }

    /// Reports whether a provider check can be submitted immediately.
    pub fn can_check<C: AppContext>(&self, cx: &C) -> bool {
        self.model.read_with(cx, |model, _| model.can_check())
    }

    /// Reports whether a verified available release can be downloaded.
    pub fn can_download<C: AppContext>(&self, cx: &C) -> bool {
        self.model.read_with(cx, |model, _| model.can_download())
    }

    /// Reports whether staged update restart can be submitted to guarded quit.
    pub fn can_restart<C: AppContext>(&self, cx: &C) -> bool {
        self.model.read_with(cx, |model, _| model.can_restart())
    }

    /// Reports whether an active check or download can be cancelled.
    pub fn can_cancel<C: AppContext>(&self, cx: &C) -> bool {
        self.model.read_with(cx, |model, _| model.can_cancel())
    }

    /// Queues an update check and redraws the bound view.
    pub fn check<V: 'static>(&self, cx: &mut Context<V>) {
        let result = self.model.read_with(cx, |model, _| {
            if model.controller.is_none() || !model.runtime_live() {
                Err(unavailable())
            } else {
                (model.check)()
            }
        });
        self.record_action(cx, &result);
    }

    /// Starts downloading the currently selected release and redraws the view.
    pub fn download<V: 'static>(&self, cx: &mut Context<V>) {
        let result = self.model.read_with(cx, |model, _| {
            if model.controller.is_none() || !model.runtime_live() {
                Err(unavailable())
            } else {
                (model.download)()
            }
        });
        self.record_action(cx, &result);
    }

    /// Requests cancellation of the active check or download.
    pub fn cancel<V: 'static>(&self, cx: &mut Context<V>) {
        let result = self.model.read_with(cx, |model, _| {
            if !model.runtime_live() {
                Err(unavailable())
            } else {
                model
                    .controller
                    .as_ref()
                    .ok_or_else(unavailable)
                    .map(|controller| controller.cancel())
            }
        });
        self.record_action(cx, &result);
    }

    /// Submits staged restart through the runtime's guarded quit boundary.
    pub fn restart<V: 'static>(&self, cx: &mut Context<V>) {
        let result = self.model.read_with(cx, |model, _| {
            if model.controller.is_none() || !model.runtime_live() {
                Err(unavailable())
            } else {
                (model.restart)()
            }
        });
        self.record_action(cx, &result);
    }

    fn record_action<V: 'static, T>(&self, cx: &mut Context<V>, result: &Result<T, Error>) {
        let (snapshot, live) = self.model.read_with(&*cx, |model, _| {
            (
                model
                    .controller
                    .as_ref()
                    .map(|controller| controller.snapshot()),
                model.runtime_live(),
            )
        });
        let error = result.as_ref().err().map(ToString::to_string);
        self.model.update(cx, |model, model_cx| {
            if live {
                if let Some(snapshot) = snapshot {
                    model.snapshot = Some(snapshot);
                }
            } else {
                model.controller = None;
                model.snapshot = None;
            }
            model.last_error = error;
            model_cx.notify();
        });
        cx.notify();
    }
}

struct BindingParts {
    controller: Option<Rc<UpdateController>>,
    snapshot: Option<UpdateSnapshot>,
    last_error: Option<String>,
    check: Action,
    download: Action,
    restart: RestartAction,
    runtime_live: RuntimeLive,
    runtime_stopped: RuntimeStopped,
}

impl BindingParts {
    fn new<R: Route>(runtime: &RuntimeContext<R>) -> Self {
        let (controller, snapshot, last_error) = match runtime.updates() {
            Ok(Some(controller)) => {
                let snapshot = controller.snapshot();
                (Some(controller), Some(snapshot), None)
            }
            Ok(None) => (None, None, None),
            Err(error) => (None, None, Some(error.to_string())),
        };

        let check_runtime = runtime.clone();
        let download_runtime = runtime.clone();
        let restart_runtime = runtime.clone();
        let live_runtime = runtime.clone();
        let stopped_runtime = runtime.clone();
        Self {
            controller,
            snapshot,
            last_error,
            check: Rc::new(move || check_runtime.check_for_updates()),
            download: Rc::new(move || download_runtime.download_update()),
            restart: Rc::new(move || restart_runtime.restart_for_update(QuitOptions::default())),
            runtime_live: Rc::new(move || live_runtime.client().is_ok()),
            runtime_stopped: Rc::new(move || {
                let runtime = stopped_runtime.clone();
                Box::pin(async move { runtime.stopped().await })
            }),
        }
    }
}

fn new_model(parts: BindingParts, cx: &mut impl AppContext) -> Entity<UpdateModel> {
    cx.new(|_| UpdateModel {
        controller: parts.controller,
        snapshot: parts.snapshot,
        last_error: parts.last_error,
        check: parts.check,
        download: parts.download,
        restart: parts.restart,
        runtime_live: parts.runtime_live,
        runtime_stopped: parts.runtime_stopped,
        observation: None,
        released: false,
    })
}

fn record_binding_error<V: 'static>(
    model: &Entity<UpdateModel>,
    cx: &mut Context<V>,
    error: TaskError,
) {
    model.update(cx, |model, model_cx| {
        // A binding without an observer must not offer actions whose future
        // progress it cannot display. A new binding can retry admission later.
        model.controller = None;
        model.snapshot = None;
        model.last_error = Some(error.to_string());
        model_cx.notify();
    });
}

fn subscriptions<V: 'static>(
    model: &Entity<UpdateModel>,
    cx: &mut Context<V>,
) -> (Subscription, Subscription) {
    let observe = cx.observe(model, |_, _, cx| cx.notify());
    let release = cx.on_release({
        let weak = model.downgrade();
        move |_, cx| {
            let _ = weak.update(cx, |model, model_cx| {
                model.released = true;
                if let Some(observation) = model.observation.take() {
                    observation.cancel();
                }
                model.controller = None;
                model.snapshot = None;
                model_cx.notify();
            });
        }
    });
    (observe, release)
}

async fn observe_managed(
    controller: Rc<UpdateController>,
    weak_model: WeakEntity<UpdateModel>,
    runtime_stopped: RuntimeStopped,
    mut revision: u64,
    cx: &mut gpui::AsyncApp,
) {
    loop {
        let changed = Box::pin(controller.changed(revision));
        let stopped = (runtime_stopped)();
        match select(changed, stopped).await {
            Either::Left((snapshot, _)) => {
                revision = snapshot.revision;
                if !publish_snapshot(&weak_model, cx, snapshot) {
                    return;
                }
            }
            Either::Right(((), _)) => {
                mark_unavailable(&weak_model, cx);
                return;
            }
        }
    }
}

async fn observe_page(
    controller: Rc<UpdateController>,
    weak_model: WeakEntity<UpdateModel>,
    run: ActivityRun,
    cx: &mut gpui::AsyncApp,
) -> Result<(), TaskError> {
    let mut snapshot = controller.snapshot();
    loop {
        match run.check() {
            Ok(()) => {}
            Err(TaskError::OwnerDisposed) => {
                mark_unavailable(&weak_model, cx);
                return Ok(());
            }
            Err(_) => return Ok(()),
        }
        let revision = snapshot.revision;
        if !publish_snapshot(&weak_model, cx, snapshot) {
            return Ok(());
        }
        snapshot = controller.changed(revision).await;
    }
}

fn publish_snapshot(
    weak_model: &WeakEntity<UpdateModel>,
    cx: &mut gpui::AsyncApp,
    snapshot: UpdateSnapshot,
) -> bool {
    weak_model
        .update(cx, |model, model_cx| {
            if model.released {
                return false;
            }
            model.snapshot = Some(snapshot);
            model_cx.notify();
            true
        })
        .unwrap_or(false)
}

fn mark_unavailable(weak_model: &WeakEntity<UpdateModel>, cx: &mut gpui::AsyncApp) {
    let _ = weak_model.update(cx, |model, model_cx| {
        if model.released {
            return;
        }
        model.controller = None;
        model.snapshot = None;
        model.last_error = Some("update runtime is no longer available".into());
        model_cx.notify();
    });
}

impl Observation {
    fn cancel(self) {
        match self {
            Self::Standalone(task) => {
                drop(task);
            }
            Self::Page(task) => {
                let _ = task.cancel();
            }
        }
    }
}

fn unavailable() -> Error {
    Error::Policy("update binding is unavailable".into())
}

fn update_info(state: &UpdateState) -> Option<&UpdateInfo> {
    match state {
        UpdateState::Available(info)
        | UpdateState::ReadyToRestart(info)
        | UpdateState::AwaitingApproval(info)
        | UpdateState::Restarting(info) => Some(info),
        UpdateState::Downloading { update, .. } | UpdateState::Extracting { update, .. } => {
            Some(update)
        }
        _ => None,
    }
}

#[cfg(all(test, feature = "view-test-support", not(target_family = "wasm")))]
#[path = "updates_binding_tests.rs"]
mod updates_binding_tests;
