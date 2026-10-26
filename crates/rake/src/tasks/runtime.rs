use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::future::Future;
use std::rc::{Rc, Weak};

use event_listener::Event;
use futures::future::{Either, LocalBoxFuture, select};
use futures::stream::FuturesUnordered;
use futures::{FutureExt, StreamExt};
use tracing::Instrument;

use crate::diagnostics::Collector;
use crate::http::CancellationToken;
use crate::router::{GenerationId, RunId, RuntimeId, TaskId, WindowId};

use super::{
    HostVisibility, TaskConfig, TaskError, TaskKind, TaskOrigin, TaskOwner, TaskSnapshot,
    TaskState, WorkSnapshot,
};

type Factory = Rc<dyn Fn(ActivityRun) -> LocalBoxFuture<'static, Result<(), TaskError>>>;
type Handler = Rc<dyn Fn(Box<dyn Any>) -> Result<(), TaskError>>;

pub(crate) struct Scope {
    core: Weak<Core>,
    owner: TaskOwner,
    alive: Cell<bool>,
    current: Cell<bool>,
    presented: Cell<bool>,
    live: Cell<bool>,
    handlers: RefCell<HashMap<TypeId, Handler>>,
}

impl Scope {
    pub(crate) fn handle(self: &Rc<Self>, origin: TaskOrigin) -> TaskScope {
        TaskScope {
            scope: Rc::downgrade(self),
            core: self.core.clone(),
            origin: Some(origin),
        }
    }

    pub(crate) fn update(self: &Rc<Self>, current: bool, presented: bool, live: bool) {
        self.current.set(current);
        self.presented.set(presented);
        self.live.set(live);

        if let Some(core) = self.core.upgrade() {
            core.reconcile(self);
        }
    }

    pub(crate) fn close(&self) {
        if !self.alive.replace(false) {
            return;
        }

        self.handlers.borrow_mut().clear();

        if let Some(core) = self.core.upgrade() {
            let ids: Vec<_> = core
                .state
                .borrow()
                .records
                .values()
                .filter(|record| record.snapshot.owner == self.owner)
                .map(|record| record.snapshot.id)
                .collect();

            for id in ids {
                core.cancel(id);
            }
        }
    }

    fn eligible(&self, kind: TaskKind, host: HostVisibility) -> bool {
        self.alive.get()
            && match kind {
                TaskKind::Task => true,
                TaskKind::Activity => {
                    self.current.get()
                        && (self.live.get()
                            || (self.presented.get() && host != HostVisibility::Hidden))
                }
                TaskKind::WhileVisible => {
                    self.current.get() && self.presented.get() && host != HostVisibility::Hidden
                }
            }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        self.close();
    }
}

struct Record {
    snapshot: TaskSnapshot,
    scope: Weak<Scope>,
    factory: Option<Factory>,
    token: Option<CancellationToken>,
    accepted: Option<Rc<Cell<bool>>>,
}

struct Publication {
    run: ActivityRun,
    handler: Handler,
    value: Box<dyn Any>,
}

struct State {
    records: BTreeMap<TaskId, Record>,
    completed: VecDeque<TaskId>,
    jobs: BTreeMap<TaskId, LocalBoxFuture<'static, ()>>,
    publications: VecDeque<Publication>,
    revision: u64,
    sequence: u64,
    run_sequence: u64,
    visibility: HostVisibility,
    windows: BTreeMap<WindowId, HostVisibility>,
    paused: bool,
    generation: GenerationId,
    closed: bool,
}

struct Core {
    state: RefCell<State>,
    event: Event,
    config: TaskConfig,
    collector: Collector,
    runtime: RuntimeId,
}

impl Core {
    fn notify(&self) {
        self.state.borrow_mut().revision += 1;
        self.event.notify(usize::MAX);
    }

    fn log(&self, record: &TaskSnapshot) {
        self.collector.record(
            "INFO",
            "rake::tasks",
            "work",
            Vec::new(),
            BTreeMap::from([
                ("runtime_id".into(), self.runtime.to_string().into()),
                ("task_id".into(), record.id.to_string().into()),
                (
                    "run_id".into(),
                    record
                        .run_id
                        .map(|id| id.to_string())
                        .unwrap_or_default()
                        .into(),
                ),
                ("owner".into(), format!("{:?}", record.owner).into()),
                (
                    "tab_id".into(),
                    record
                        .origin
                        .tab_id
                        .map(|id| id.to_string())
                        .unwrap_or_default()
                        .into(),
                ),
                (
                    "entry_id".into(),
                    record
                        .origin
                        .entry_id
                        .map(|id| id.to_string())
                        .unwrap_or_default()
                        .into(),
                ),
                (
                    "generation_id".into(),
                    record.generation_id.to_string().into(),
                ),
                ("state".into(), format!("{:?}", record.state).into()),
            ]),
        );
    }

    fn trim(&self, state: &mut State, admitting: bool) {
        while state.completed.len() > self.config.completed
            || (admitting && state.records.len() >= self.config.capacity)
        {
            let Some(id) = state.completed.pop_front() else {
                break;
            };

            state.records.remove(&id);
        }
    }

    fn cancel(&self, id: TaskId) -> bool {
        let mut state = self.state.borrow_mut();

        let Some(record) = state.records.get_mut(&id) else {
            return false;
        };

        if record.factory.is_none()
            && record
                .token
                .as_ref()
                .is_none_or(CancellationToken::is_cancelled)
        {
            return false;
        }

        let factory = record.factory.take();

        record.snapshot.eligible = false;

        if let Some(accepted) = &record.accepted {
            accepted.set(false);
        }

        if let Some(token) = &record.token {
            token.cancel();
            record.snapshot.state = TaskState::Cancelling;
        } else {
            record.snapshot.state = TaskState::Cancelled;

            if !state.completed.contains(&id) {
                state.completed.push_back(id);
            }
        }

        self.trim(&mut state, false);
        drop(state);
        drop(factory);
        self.notify();

        true
    }

    fn reconcile(self: &Rc<Self>, scope: &Rc<Scope>) {
        let ids: Vec<_> = self
            .state
            .borrow()
            .records
            .values()
            .filter(|record| record.snapshot.owner == scope.owner && record.factory.is_some())
            .map(|record| record.snapshot.id)
            .collect();

        for id in ids {
            let mut state = self.state.borrow_mut();

            if state.closed || state.paused {
                return;
            }

            let window = state
                .records
                .get(&id)
                .expect("registered activity")
                .snapshot
                .origin
                .window_id;

            let host = state
                .windows
                .get(&window)
                .copied()
                .unwrap_or(if window == WindowId(1) {
                    state.visibility
                } else {
                    HostVisibility::Unknown
                });

            let record = state.records.get_mut(&id).expect("registered activity");
            let eligible = scope.eligible(record.snapshot.kind, host);

            if record.snapshot.eligible == eligible {
                continue;
            }

            record.snapshot.eligible = eligible;

            if !eligible {
                if let Some(accepted) = &record.accepted {
                    accepted.set(false);
                }

                if let Some(token) = &record.token {
                    token.cancel();
                    record.snapshot.state = TaskState::Cancelling;
                }

                drop(state);
                self.notify();
            } else {
                let factory = record.factory.clone().expect("registered factory");

                drop(state);
                self.start(id, move |run| factory(run));
            }
        }
    }

    fn start(
        self: &Rc<Self>,
        id: TaskId,
        factory: impl FnOnce(ActivityRun) -> LocalBoxFuture<'static, Result<(), TaskError>> + 'static,
    ) {
        let mut state = self.state.borrow_mut();

        state.run_sequence += 1;
        let run_id = RunId(state.run_sequence);
        let generation = state.generation;
        let record = state.records.get_mut(&id).expect("admitted task");

        if let Some(previous) = &record.accepted {
            previous.set(false);
        }

        let token = CancellationToken::new();
        let accepted = Rc::new(Cell::new(true));

        record.snapshot.run_id = Some(run_id);
        record.snapshot.generation_id = generation;
        record.snapshot.state = TaskState::Queued;
        record.token = Some(token.clone());
        record.accepted = Some(accepted.clone());
        let run = ActivityRun {
            core: Rc::downgrade(self),
            scope: record.scope.clone(),
            task_id: id,
            run_id,
            generation,
            token,
            accepted,
        };

        let span = tracing::info_span!("work.run", runtime_id = %self.runtime, task_id = %id,
            run_id = %run_id, generation_id = %generation, owner = ?record.snapshot.owner,
            tab_id = tracing::field::Empty, entry_id = tracing::field::Empty);

        if let Some(tab) = record.snapshot.origin.tab_id {
            span.record("tab_id", tab.to_string());
        }

        if let Some(entry) = record.snapshot.origin.entry_id {
            span.record("entry_id", entry.to_string());
        }

        let job = async move {
            if let Some(core) = run.core.upgrade() {
                core.set_running(&run);
            }

            let result = if run.validate().is_err() {
                TaskState::Cancelled
            } else {
                let future = factory(run.clone());

                match select(Box::pin(run.token.cancelled()), future).await {
                    Either::Left(_) => TaskState::Cancelled,
                    Either::Right((Ok(()), _)) => TaskState::Completed,
                    Either::Right((Err(error), _)) => TaskState::Failed(error),
                }
            };

            if let Some(core) = run.core.upgrade() {
                core.finish(&run, result);
            }
        }
        .instrument(span)
        .boxed_local();

        let obsolete = state.jobs.insert(id, job);
        let snapshot = state.records[&id].snapshot.clone();

        drop(state);
        drop(obsolete);
        self.log(&snapshot);
        self.notify();
    }

    fn set_running(&self, run: &ActivityRun) {
        let mut state = self.state.borrow_mut();

        if let Some(record) = state.records.get_mut(&run.task_id)
            && record.snapshot.run_id == Some(run.run_id)
            && run.accepted.get()
            && !run.token.is_cancelled()
        {
            record.snapshot.state = TaskState::Running;
        }

        drop(state);
        self.notify();
    }

    fn finish(&self, run: &ActivityRun, result: TaskState) {
        let mut state = self.state.borrow_mut();

        let Some(record) = state.records.get_mut(&run.task_id) else {
            return;
        };

        if record.snapshot.run_id != Some(run.run_id) {
            return;
        }

        // A queued publication remains valid after successful future completion.
        if !matches!(result, TaskState::Completed) {
            run.accepted.set(false);
        }

        if !matches!(record.snapshot.state, TaskState::Failed(_)) {
            record.snapshot.state = result;
        }

        record.token = None;
        let snapshot = record.snapshot.clone();

        if record.factory.is_none() && !state.completed.contains(&run.task_id) {
            state.completed.push_back(run.task_id);
        }

        self.trim(&mut state, false);
        drop(state);
        self.log(&snapshot);
        self.notify();
    }
}

/// Weak handle to one owner. Cloning it does not keep that owner alive.
#[derive(Clone)]
pub struct TaskScope {
    scope: Weak<Scope>,
    core: Weak<Core>,
    origin: Option<TaskOrigin>,
}

impl TaskScope {
    pub(crate) fn unavailable() -> Self {
        Self {
            scope: Weak::new(),
            core: Weak::new(),
            origin: None,
        }
    }

    /// Eagerly submits one local future. Hiding its page never restarts it.
    pub fn spawn(
        &self,
        label: impl Into<String>,
        future: impl Future<Output = Result<(), TaskError>> + 'static,
    ) -> Result<TaskHandle, TaskError> {
        self.spawn_with(label, move |_| future)
    }

    /// Submits one future with cancellation, identity and checked publication context.
    pub fn spawn_with<F, Fut>(
        &self,
        label: impl Into<String>,
        factory: F,
    ) -> Result<TaskHandle, TaskError>
    where
        F: FnOnce(ActivityRun) -> Fut + 'static,
        Fut: Future<Output = Result<(), TaskError>> + 'static,
    {
        let handle = self.register(label.into(), TaskKind::Task, None)?;
        let core = self.core.upgrade().ok_or(TaskError::OwnerDisposed)?;

        core.start(handle.id, move |run| Box::pin(factory(run)));

        Ok(handle)
    }

    pub(crate) fn activity<F, Fut>(
        &self,
        key: String,
        kind: TaskKind,
        factory: F,
    ) -> Result<TaskHandle, TaskError>
    where
        F: Fn(ActivityRun) -> Fut + 'static,
        Fut: Future<Output = Result<(), TaskError>> + 'static,
    {
        let factory: Factory = Rc::new(move |run| Box::pin(factory(run)));
        let handle = self.register(key, kind, Some(factory))?;
        let scope = self.scope.upgrade().ok_or(TaskError::OwnerDisposed)?;

        self.core
            .upgrade()
            .ok_or(TaskError::OwnerDisposed)?
            .reconcile(&scope);

        Ok(handle)
    }

    fn register(
        &self,
        key: String,
        kind: TaskKind,
        factory: Option<Factory>,
    ) -> Result<TaskHandle, TaskError> {
        let core = self.core.upgrade().ok_or(TaskError::OwnerDisposed)?;

        let scope = self
            .scope
            .upgrade()
            .filter(|scope| scope.alive.get())
            .ok_or(TaskError::OwnerDisposed)?;

        let mut state = core.state.borrow_mut();

        if state.closed {
            return Err(TaskError::OwnerDisposed);
        }

        if state.paused && factory.is_none() {
            return Err(TaskError::Reloading);
        }

        if key.trim().is_empty() {
            return Err(TaskError::InvalidLabel);
        }

        if factory.is_some()
            && state.records.values().any(|record| {
                record.snapshot.owner == scope.owner
                    && record.snapshot.key == key
                    && record.factory.is_some()
            })
        {
            return Err(TaskError::DuplicateResource);
        }

        core.trim(&mut state, true);

        if state.records.len() >= core.config.capacity {
            return Err(TaskError::Capacity);
        }

        state.sequence += 1;
        let id = TaskId(state.sequence);

        let generation = state.generation;

        state.records.insert(
            id,
            Record {
                snapshot: TaskSnapshot {
                    id,
                    key,
                    owner: scope.owner,
                    origin: self.origin.clone().ok_or(TaskError::OwnerDisposed)?,
                    kind,
                    run_id: None,
                    generation_id: generation,
                    eligible: false,
                    state: TaskState::Registered,
                },
                scope: Rc::downgrade(&scope),
                factory,
                token: None,
                accepted: None,
            },
        );
        drop(state);
        core.notify();

        Ok(TaskHandle {
            core: Rc::downgrade(&core),
            id,
        })
    }

    /// Registers one typed event handler in this scope, replacing any previous handler.
    /// Delivery runs on the host driver, after rechecking owner and run identity.
    pub fn on_event<T: 'static>(
        &self,
        handler: impl Fn(T) -> Result<(), TaskError> + 'static,
    ) -> Result<(), TaskError> {
        let scope = self
            .scope
            .upgrade()
            .filter(|scope| scope.alive.get())
            .ok_or(TaskError::OwnerDisposed)?;

        scope.handlers.borrow_mut().insert(
            TypeId::of::<T>(),
            Rc::new(move |value| {
                handler(*value.downcast::<T>().expect("handler selected by TypeId"))
            }),
        );

        Ok(())
    }
}

/// Application scope with the caller's immutable causal origin.
#[derive(Clone)]
pub struct AppHandle(pub(crate) TaskScope);

impl AppHandle {
    /// Work survives closing its source tab, ending with this application runtime.
    pub fn tasks(&self) -> TaskScope {
        self.0.clone()
    }
}

/// Observation and explicit cancellation handle. Dropping it does not cancel work.
#[derive(Clone)]
pub struct TaskHandle {
    core: Weak<Core>,
    id: TaskId,
}

impl TaskHandle {
    /// Stable task or factory registration identity.
    pub fn id(&self) -> TaskId {
        self.id
    }

    /// Cancels work and unregisters an activity factory, preventing future restarts.
    /// Returns false if there is no remaining registration/run to cancel, including
    /// a run whose cancellation was already requested.
    pub fn cancel(&self) -> Result<bool, TaskError> {
        Ok(self
            .core
            .upgrade()
            .ok_or(TaskError::OwnerDisposed)?
            .cancel(self.id))
    }

    /// Copies the latest run state. Completed records can expire under retention bounds.
    pub fn snapshot(&self) -> Result<TaskSnapshot, TaskError> {
        self.core
            .upgrade()
            .ok_or(TaskError::OwnerDisposed)?
            .state
            .borrow()
            .records
            .get(&self.id)
            .map(|record| record.snapshot.clone())
            .ok_or(TaskError::Expired)
    }

    /// Waits for the latest run to finish; activity eligibility can later create a new run.
    pub async fn wait(&self) -> Result<TaskState, TaskError> {
        loop {
            let core = self.core.upgrade().ok_or(TaskError::OwnerDisposed)?;
            let listener = core.event.listen();
            let snapshot = self.snapshot()?;

            if matches!(
                snapshot.state,
                TaskState::Completed | TaskState::Cancelled | TaskState::Failed(_)
            ) {
                return Ok(snapshot.state);
            }

            drop(core);
            listener.await;
        }
    }
}

/// One fresh resource run. Publication is checked both at submission and delivery.
#[derive(Clone)]
pub struct ActivityRun {
    core: Weak<Core>,
    scope: Weak<Scope>,
    task_id: TaskId,
    run_id: RunId,
    generation: GenerationId,
    token: CancellationToken,
    accepted: Rc<Cell<bool>>,
}

impl ActivityRun {
    /// Identifies this execution, changing on every factory restart.
    pub fn id(&self) -> RunId {
        self.run_id
    }

    /// Cancellation token usable with Rake HTTP and cooperative application work.
    pub fn cancellation(&self) -> CancellationToken {
        self.token.clone()
    }

    /// Queues a typed event. Success means admitted, not that delivery already happened.
    pub fn publish<T: 'static>(&self, value: T) -> Result<(), TaskError> {
        let core = self.validate()?;

        if core
            .state
            .borrow()
            .records
            .get(&self.task_id)
            .is_none_or(|record| {
                record.snapshot.run_id != Some(self.run_id) || record.token.is_none()
            })
        {
            return Err(TaskError::StaleRun);
        }

        let scope = self.scope.upgrade().ok_or(TaskError::OwnerDisposed)?;

        let handler = scope
            .handlers
            .borrow()
            .get(&TypeId::of::<T>())
            .cloned()
            .ok_or(TaskError::UnregisteredEvent)?;

        let mut state = core.state.borrow_mut();

        if state.publications.len() >= core.config.publications {
            return Err(TaskError::PublicationQueueFull);
        }

        state.publications.push_back(Publication {
            run: self.clone(),
            handler,
            value: Box::new(value),
        });
        drop(state);
        core.notify();

        Ok(())
    }

    fn validate(&self) -> Result<Rc<Core>, TaskError> {
        let core = self.core.upgrade().ok_or(TaskError::OwnerDisposed)?;

        if core.state.borrow().closed || self.scope.upgrade().is_none_or(|scope| !scope.alive.get())
        {
            return Err(TaskError::OwnerDisposed);
        }

        if !self.accepted.get()
            || self.token.is_cancelled()
            || self.generation != core.state.borrow().generation
            || core.state.borrow().paused
        {
            return Err(TaskError::StaleRun);
        }

        Ok(core)
    }
}

pub(crate) struct WorkRuntime {
    core: Rc<Core>,
    app: Rc<Scope>,
}

struct WorkDriver<'a>(&'a WorkRuntime);

impl Drop for WorkDriver<'_> {
    fn drop(&mut self) {
        self.0.shutdown();
        let mut state = self.0.core.state.borrow_mut();
        let jobs = std::mem::take(&mut state.jobs);
        let publications = std::mem::take(&mut state.publications);

        for record in state.records.values_mut() {
            if matches!(
                record.snapshot.state,
                TaskState::Queued
                    | TaskState::Running
                    | TaskState::Cancelling
                    | TaskState::Registered
            ) {
                record.snapshot.state = TaskState::Cancelled;
                record.token = None;
            }
        }

        let terminal: Vec<_> = state
            .records
            .values()
            .filter(|record| record.factory.is_none())
            .map(|record| record.snapshot.id)
            .collect();

        for id in terminal {
            if !state.completed.contains(&id) {
                state.completed.push_back(id);
            }
        }

        self.0.core.trim(&mut state, false);
        drop(state);
        drop(jobs);
        drop(publications);
        self.0.core.notify();
    }
}

impl WorkRuntime {
    pub(crate) fn new(runtime: RuntimeId, config: TaskConfig, collector: Collector) -> Self {
        let core = Rc::new(Core {
            state: RefCell::new(State {
                records: BTreeMap::new(),
                completed: VecDeque::new(),
                jobs: BTreeMap::new(),
                publications: VecDeque::new(),
                revision: 0,
                sequence: 0,
                run_sequence: 0,
                visibility: HostVisibility::Unknown,
                windows: BTreeMap::new(),
                paused: false,
                generation: GenerationId(1),
                closed: false,
            }),
            event: Event::new(),
            config,
            collector,
            runtime,
        });

        let app = Self::scope_for(&core, TaskOwner::Application);

        Self { core, app }
    }

    fn scope_for(core: &Rc<Core>, owner: TaskOwner) -> Rc<Scope> {
        Rc::new(Scope {
            core: Rc::downgrade(core),
            owner,
            alive: Cell::new(true),
            current: Cell::new(false),
            presented: Cell::new(false),
            live: Cell::new(false),
            handlers: RefCell::new(HashMap::new()),
        })
    }

    pub(crate) fn scope(&self, owner: TaskOwner) -> Rc<Scope> {
        Self::scope_for(&self.core, owner)
    }

    pub(crate) fn origin(&self) -> TaskOrigin {
        TaskOrigin {
            runtime_id: self.core.runtime,
            window_id: WindowId(1),
            tab_id: None,
            entry_id: None,
        }
    }

    pub(crate) fn app(&self, origin: TaskOrigin) -> AppHandle {
        AppHandle(self.app.handle(origin))
    }

    pub(crate) fn cancel(&self, id: TaskId) -> Result<bool, TaskError> {
        if !self.core.state.borrow().records.contains_key(&id) {
            return Err(TaskError::Expired);
        }

        Ok(self.core.cancel(id))
    }

    pub(crate) fn snapshot(&self) -> WorkSnapshot {
        let state = self.core.state.borrow();

        WorkSnapshot {
            revision: state.revision,
            host_visibility: state.visibility,
            tasks: state
                .records
                .values()
                .map(|record| record.snapshot.clone())
                .collect(),
            pending_publications: state.publications.len(),
        }
    }

    pub(crate) fn revision(&self) -> u64 {
        self.core.state.borrow().revision
    }

    pub(crate) async fn changed(&self, revision: u64) {
        loop {
            let listener = self.core.event.listen();

            if self.revision() != revision {
                return;
            }

            listener.await;
        }
    }

    pub(crate) fn forget_window(&self, window: WindowId) {
        self.core.state.borrow_mut().windows.remove(&window);
    }

    pub(crate) fn window_visibility(&self, window: WindowId, visibility: HostVisibility) {
        let mut state = self.core.state.borrow_mut();

        if state.windows.get(&window) == Some(&visibility) || state.closed {
            return;
        }

        state.windows.insert(window, visibility);

        if window == WindowId(1) {
            state.visibility = visibility;
        }

        let scopes: Vec<_> = state
            .records
            .values()
            .filter_map(|record| record.scope.upgrade())
            .collect();

        drop(state);
        tracing::info!(target: "rake::tasks", ?visibility, "Host visibility changed");

        for scope in scopes {
            self.core.reconcile(&scope);
        }

        self.core.notify();
    }

    pub(crate) fn pause(&self) {
        let mut state = self.core.state.borrow_mut();

        state.paused = true;
        let publications = std::mem::take(&mut state.publications);

        for record in state.records.values_mut() {
            record.snapshot.eligible = false;

            if let Some(accepted) = &record.accepted {
                accepted.set(false);
            }

            if let Some(token) = &record.token {
                token.cancel();
                record.snapshot.state = TaskState::Cancelling;
            }
        }

        drop(state);
        drop(publications);
        self.core.notify();
    }

    pub(crate) fn quiescent(&self) -> bool {
        self.core
            .state
            .borrow()
            .records
            .values()
            .all(|record| record.token.is_none())
    }

    pub(crate) fn generation(&self) -> GenerationId {
        self.core.state.borrow().generation
    }

    pub(crate) fn run_sequence(&self) -> u64 {
        self.core.state.borrow().run_sequence
    }

    pub(crate) fn resume(&self, generation: GenerationId, run_sequence: u64) {
        let mut state = self.core.state.borrow_mut();

        state.paused = false;
        state.generation = generation;
        state.run_sequence = state.run_sequence.max(run_sequence);
        let scopes: Vec<_> = state
            .records
            .values()
            .filter_map(|record| record.scope.upgrade())
            .collect();

        drop(state);

        for scope in scopes {
            self.core.reconcile(&scope);
        }

        self.core.notify();
    }

    pub(crate) fn shutdown(&self) {
        self.core.state.borrow_mut().closed = true;
        self.app.close();
        let ids: Vec<_> = self.core.state.borrow().records.keys().copied().collect();

        for id in ids {
            self.core.cancel(id);
        }

        self.core.notify();
    }

    pub(crate) async fn run(&self) {
        let _driver = WorkDriver(self);
        let mut running = FuturesUnordered::new();

        loop {
            let listener = self.core.event.listen();

            let (jobs, publications, closed) = {
                let mut state = self.core.state.borrow_mut();

                (
                    std::mem::take(&mut state.jobs),
                    std::mem::take(&mut state.publications),
                    state.closed,
                )
            };

            running.extend(jobs.into_values());
            let delivered = !publications.is_empty();

            for publication in publications {
                if publication.run.validate().is_ok()
                    && let Err(error) = (publication.handler)(publication.value)
                {
                    publication.run.token.cancel();
                    publication.run.accepted.set(false);
                    self.core.finish(&publication.run, TaskState::Failed(error));
                }
            }

            if delivered {
                self.core.notify();
            }

            if closed && running.is_empty() {
                return;
            }

            if running.is_empty() {
                listener.await;
            } else {
                let _ = select(Box::pin(running.next()), Box::pin(listener)).await;
            }
        }
    }
}

impl Drop for WorkRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}
