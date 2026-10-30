//! Local operation scheduler and the authoritative router state owner.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::Future;
use std::rc::{Rc, Weak};

use event_listener::Event;
use futures::channel::oneshot;
use futures::future::{Either, LocalBoxFuture, select};
use futures::stream::FuturesUnordered;
use futures::{FutureExt, StreamExt};
use tracing::Instrument;

use crate::diagnostics::Collector;
use crate::http::CancellationToken;

use crate::tasks::{
    ActivityRun, AppHandle, HostVisibility, Scope, TaskError, TaskHandle, TaskKind, TaskOwner,
    TaskScope, WorkRuntime,
};

use super::{
    Activation, BlockReason, BrowserCheckpoint, CancelReason, ChoiceAction, Closed, DecisionChoice,
    DecisionError, DecisionId, DecisionRequest, DecisionSnapshot, DisposeReason, Entered, EntryId,
    EntrySnapshot, GuardContext, GuardKind, GuardResult, HistoryChange, ModeChanged, MountError,
    OpenOptions, OpenTarget, Opened, Operation, OperationError, OperationId, OperationState,
    Outcome, RevealScope, Revealed, Route, RouteDecodeError, RouterConfig, RouterSnapshot,
    RuntimeId, Selected, TabId, TabMode, TabSnapshot, WindowClosed, WindowHost, WindowId,
    WindowSnapshot,
};

mod quit;
mod reload;
mod windows;

#[cfg(test)]
mod tests;

pub(crate) type Guard<R> = Rc<dyn Fn(GuardContext<R>) -> LocalBoxFuture<'static, GuardResult>>;
pub(crate) type Resource = Rc<dyn Any>;

#[cfg(feature = "gpui")]
type WindowContexts = Rc<RefCell<BTreeMap<WindowId, gpui::AsyncWindowContext>>>;

pub(crate) type Mount<R> = Rc<
    dyn Fn(
        R,
        EntryHandle<R>,
        Navigator<R>,
    ) -> LocalBoxFuture<'static, Result<Resource, MountError>>,
>;

type ReloadCapture = Rc<dyn Fn() -> Result<crate::reload::Checkpoint, crate::reload::ReloadError>>;

type HistorySink = Rc<dyn Fn(HistoryChange) -> Result<(), String>>;

struct Entry<R: Route> {
    scope: Rc<Scope>,
    tab_scope: Rc<Scope>,
    id: EntryId,
    tab: TabId,
    window: WindowId,
    destination: R,
    revision: Cell<u64>,
    dirty: Cell<bool>,
    alive: Cell<bool>,
    guards: RefCell<Vec<Guard<R>>>,
    resource: RefCell<Option<Resource>>,
    cancellation: CancellationToken,
    reload_capture: RefCell<Option<ReloadCapture>>,
    restored_state: RefCell<Option<crate::reload::Checkpoint>>,
    state_consumed: Cell<bool>,
}

impl<R: Route> Entry<R> {
    fn dispose(&self) {
        self.scope.close();
        self.alive.set(false);
        self.cancellation.cancel();
    }
}

impl<R: Route> Drop for Entry<R> {
    fn drop(&mut self) {
        self.dispose();
    }
}

struct Tab<R: Route> {
    scope: Rc<Scope>,
    id: TabId,
    window: WindowId,
    entries: Vec<Rc<Entry<R>>>,
    cursor: usize,
    mode: TabMode,
    selected_at: u64,
}

impl<R: Route> Tab<R> {
    fn current(&self) -> Rc<Entry<R>> {
        self.entries[self.cursor].clone()
    }
}

struct PendingDecision<R: Route> {
    snapshot: DecisionSnapshot,
    entry: Weak<Entry<R>>,
    revision: u64,
    browser_epoch: Option<u64>,
    sender: oneshot::Sender<Option<DecisionChoice>>,
    choices: Vec<DecisionChoice>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Lane {
    Workspace,
    Tab(TabId),
    Window(WindowId),
}

struct Job {
    lanes: Vec<Lane>,
    token: CancellationToken,
    future: LocalBoxFuture<'static, ()>,
}

struct WindowState {
    selected: Option<TabId>,
    closing: Option<OperationId>,
    opening: Option<OperationId>,
    visibility: HostVisibility,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            selected: None,
            closing: None,
            opening: None,
            visibility: HostVisibility::Unknown,
        }
    }
}

struct State<R: Route> {
    windows: BTreeMap<WindowId, WindowState>,
    tabs: Vec<Tab<R>>,
    sequence: u64,
    operation_sequence: u64,
    revision: u64,
    browser_epoch: u64,
    history_initialized: bool,
    history_error: Option<String>,
    history_sink: Option<HistorySink>,
    queue: VecDeque<Job>,
    operations: BTreeMap<OperationId, OperationState>,
    results: BTreeMap<OperationId, serde_json::Value>,
    tokens: BTreeMap<OperationId, CancellationToken>,
    completed: VecDeque<OperationId>,
    decisions: BTreeMap<DecisionId, PendingDecision<R>>,
}

impl<R: Route> State<R> {
    fn advance_browser(&mut self) {
        self.browser_epoch += 1;
        let ids: Vec<_> = self
            .decisions
            .iter()
            .filter(|(_, decision)| decision.browser_epoch.is_some())
            .map(|(id, _)| *id)
            .collect();

        for id in ids {
            if let Some(decision) = self.decisions.remove(&id) {
                let _ = decision.sender.send(None);
            }
        }
    }

    fn next(&mut self) -> u64 {
        self.sequence += 1;

        self.sequence
    }

    fn tab(&self, id: TabId) -> Result<&Tab<R>, OperationError> {
        self.tabs
            .iter()
            .find(|tab| tab.id == id)
            .ok_or(OperationError::ClosedTarget)
    }

    fn tab_mut(&mut self, id: TabId) -> Result<&mut Tab<R>, OperationError> {
        self.tabs
            .iter_mut()
            .find(|tab| tab.id == id)
            .ok_or(OperationError::ClosedTarget)
    }

    fn select(&mut self, id: TabId) {
        let tick = self.next();

        let window = self.tab(id).expect("selected tab exists").window;

        self.windows
            .get_mut(&window)
            .expect("tab host exists")
            .selected = Some(id);

        if let Ok(tab) = self.tab_mut(id) {
            tab.selected_at = tick;
        }
    }

    fn matching(&self, destination: &R, window: Option<WindowId>) -> Option<Rc<Entry<R>>> {
        let address = destination.encode();

        self.tabs
            .iter()
            .filter(|tab| {
                window.is_none_or(|window| tab.window == window)
                    && tab.current().destination.encode() == address
            })
            .max_by_key(|tab| tab.selected_at)
            .map(Tab::current)
    }

    fn invalidate(&mut self, entry: Option<EntryId>) {
        let ids: Vec<_> = self
            .decisions
            .iter()
            .filter(|(_, decision)| entry.is_none_or(|entry| decision.snapshot.entry_id == entry))
            .map(|(id, _)| *id)
            .collect();

        for id in ids {
            if let Some(decision) = self.decisions.remove(&id) {
                let _ = decision.sender.send(None);
            }
        }
    }
}

struct Core<R: Route> {
    tasks: WorkRuntime,
    quit: RefCell<crate::quit::QuitStatus>,
    quit_handle: RefCell<Option<Operation<crate::quit::QuitResult>>>,
    reload: RefCell<crate::reload::ReloadStatus>,
    reload_ui_ready: RefCell<BTreeSet<WindowId>>,
    #[cfg(target_family = "wasm")]
    page_reload: Cell<bool>,
    control: Rc<crate::control::ClientState>,
    #[cfg(feature = "gpui")]
    host: RefCell<BTreeMap<WindowId, crate::tasks::host::HostObserver>>,
    #[cfg(feature = "gpui")]
    view_contexts: RefCell<Option<WindowContexts>>,
    state: RefCell<State<R>>,
    runtime: RuntimeId,
    initial_window: Cell<WindowId>,
    config: RouterConfig,
    collector: Collector,
    mount: Mount<R>,
    window_host: RefCell<Option<Rc<dyn WindowHost>>>,
    window_closes: RefCell<BTreeMap<WindowId, Operation<WindowClosed>>>,
    guards: RefCell<Vec<Guard<R>>>,
    event: Event,
    work: Event,
    driving: Cell<bool>,
    has_window: Cell<bool>,
    shutdown: Cell<bool>,
}

impl<R: Route> Core<R> {
    fn notify(&self) {
        self.state.borrow_mut().revision += 1;
        self.event.notify(usize::MAX);
    }

    fn status(&self, id: OperationId, status: OperationState) {
        self.state.borrow_mut().operations.insert(id, status);
        self.notify();
    }

    fn event(&self, id: OperationId, phase: &str) {
        self.collector.record(
            "INFO",
            "rake::router",
            "operation",
            Vec::new(),
            BTreeMap::from([
                ("operation_id".into(), id.to_string().into()),
                ("runtime_id".into(), self.runtime.to_string().into()),
                ("phase".into(), phase.into()),
            ]),
        );
    }

    fn finish<T: serde::Serialize>(
        &self,
        id: OperationId,
        result: &Result<Outcome<T>, OperationError>,
    ) {
        let status = match result {
            Ok(Outcome::Committed(_)) => OperationState::Committed,
            Ok(Outcome::Cancelled(reason)) => OperationState::Cancelled(reason.clone()),
            Ok(Outcome::Blocked(reason)) => OperationState::Blocked(reason.clone()),
            Err(error) => OperationState::Failed(error.clone()),
        };

        let mut state = self.state.borrow_mut();

        let decisions: Vec<_> = state
            .decisions
            .iter()
            .filter(|(_, pending)| pending.snapshot.operation_id == id)
            .map(|(id, _)| *id)
            .collect();

        for decision in decisions {
            state.decisions.remove(&decision);
        }

        if let Ok(Outcome::Committed(value)) = result {
            state.results.insert(
                id,
                serde_json::to_value(value).expect("framework operation result is serializable"),
            );
        }

        state.tokens.remove(&id);
        state.operations.insert(id, status);
        state.completed.push_back(id);

        while state.completed.len() > self.config.completed_operations {
            if let Some(expired) = state.completed.pop_front() {
                state.operations.remove(&expired);
                state.results.remove(&expired);
            }
        }

        drop(state);
        self.event(id, "finished");
        self.notify();
    }

    fn emit_history(&self, replace: bool) {
        let mut state = self.state.borrow_mut();

        let change = state
            .windows
            .get(&self.initial_window.get())
            .and_then(|window| window.selected)
            .and_then(|id| state.tab(id).ok())
            .map(|tab| {
                let entry = tab.current();

                let checkpoint = BrowserCheckpoint {
                    version: 1,
                    runtime_id: self.runtime,
                    tab_id: tab.id,
                    entry_id: entry.id,
                    destination: entry.destination.encode(),
                };

                let uri = entry
                    .destination
                    .to_uri()
                    .expect("destinations validated before commit");

                if replace || !state.history_initialized {
                    HistoryChange::Replace(checkpoint, uri)
                } else {
                    HistoryChange::Push(checkpoint, uri)
                }
            })
            .unwrap_or(HistoryChange::Clear);

        state.history_initialized = true;
        let sink = state.history_sink.clone();

        drop(state);

        if let Some(sink) = sink {
            let error = sink(change).err();

            self.state.borrow_mut().history_error = error;
            self.notify();
        }
    }
}

/// Owns authoritative state, retained resources and an explicitly driven local scheduler.
#[derive(Clone)]
pub struct Router<R: Route>(Rc<Core<R>>);

impl<R: Route> Router<R> {
    /// Creates a headless routing runtime. GPUI applications install their real mount adapter.
    pub fn new(config: RouterConfig) -> Self {
        Self::with_mount(
            config,
            Collector::default(),
            Rc::new(|_, _, _| Box::pin(async { Ok(Rc::new(()) as Resource) })),
        )
    }

    /// Creates a headless runtime with a fixed typed application composition.
    pub fn with_services(config: RouterConfig, services: crate::services::ServiceRegistry) -> Self {
        let router = Self::new(config);

        router.client().set_services(services);

        router
    }

    pub(crate) fn with_mount(config: RouterConfig, collector: Collector, mount: Mount<R>) -> Self {
        let runtime = RuntimeId(uuid::Uuid::new_v4());
        let tasks = WorkRuntime::new(runtime, config.tasks.clone(), collector.clone());

        Self(Rc::new(Core {
            tasks,
            reload: RefCell::new(crate::reload::ReloadStatus {
                restored_from: None,
                generation: super::GenerationId(1),
                phase: crate::reload::ReloadPhase::Idle,
                request: None,
                error: None,
            }),
            reload_ui_ready: RefCell::new(BTreeSet::new()),
            #[cfg(target_family = "wasm")]
            page_reload: Cell::new(false),
            control: Rc::new(crate::control::ClientState::new(config.control_requests)),
            #[cfg(feature = "gpui")]
            host: RefCell::new(BTreeMap::new()),
            #[cfg(feature = "gpui")]
            view_contexts: RefCell::new(None),
            state: RefCell::new(State {
                windows: BTreeMap::from([(WindowId(1), WindowState::default())]),
                tabs: Vec::new(),
                sequence: 0,
                operation_sequence: 0,
                revision: 0,
                browser_epoch: 0,
                history_initialized: false,
                history_error: None,
                history_sink: None,
                queue: VecDeque::new(),
                operations: BTreeMap::new(),
                results: BTreeMap::new(),
                tokens: BTreeMap::new(),
                completed: VecDeque::new(),
                decisions: BTreeMap::new(),
            }),
            runtime,
            initial_window: Cell::new(WindowId(1)),
            config,
            collector,
            mount,
            window_host: RefCell::new(None),
            window_closes: RefCell::new(BTreeMap::new()),
            guards: RefCell::new(Vec::new()),
            event: Event::new(),
            work: Event::new(),
            driving: Cell::new(false),
            quit: RefCell::new(crate::quit::QuitStatus::default()),
            quit_handle: RefCell::new(None),
            has_window: Cell::new(false),
            shutdown: Cell::new(false),
        }))
    }

    /// Identity of this live runtime, stable until host teardown or full reload.
    pub fn runtime_id(&self) -> RuntimeId {
        self.0.runtime
    }

    pub(crate) fn operation_limits(&self) -> (usize, usize) {
        (
            self.0.config.completed_operations,
            self.0.config.pending_operations,
        )
    }

    /// Returns a window-bound command proxy; screen proxies additionally bind a tab.
    pub fn navigator(&self) -> Navigator<R> {
        Navigator {
            core: Rc::downgrade(&self.0),
            window: self.0.initial_window.get(),
            tab: None,
            origin_entry: None,
            origin_scope: Weak::new(),
        }
    }

    /// Registers an access guard before the operation driver starts.
    pub fn entry_guard<F, Fut>(&self, guard: F) -> Result<(), OperationError>
    where
        F: Fn(GuardContext<R>) -> Fut + 'static,
        Fut: Future<Output = GuardResult> + 'static,
    {
        if self.0.driving.get() {
            return Err(OperationError::InvalidOptions);
        }

        self.0
            .guards
            .borrow_mut()
            .push(Rc::new(move |context| Box::pin(guard(context))));

        Ok(())
    }

    /// Installs the browser/embedding history sink. Failures are exposed in snapshots.
    pub fn on_history(&self, sink: impl Fn(HistoryChange) -> Result<(), String> + 'static) {
        self.0.state.borrow_mut().history_sink = Some(Rc::new(sink));
    }

    /// Copies state without exposing retained views or mutable histories.
    pub fn snapshot(&self) -> RouterSnapshot {
        self.snapshot_filtered(None)
    }

    fn snapshot_filtered(&self, window: Option<WindowId>) -> RouterSnapshot {
        let state = self.0.state.borrow();
        let window_id = window.unwrap_or(self.0.initial_window.get());
        let host = state.windows.get(&window_id);
        let mut work = self.0.tasks.snapshot();

        work.host_visibility = host.map_or(HostVisibility::Unknown, |host| host.visibility);
        let quit = self.0.quit_status(&work);

        RouterSnapshot {
            quit,
            reload: self.reload_status(),
            work,
            supported_targets: self.supported_targets(),
            windows: state.window_snapshots(),
            runtime_id: self.0.runtime,
            window_id,
            revision: state.revision + self.0.tasks.revision(),
            selected: host.and_then(|host| host.selected),
            tabs: state.tab_snapshots(window),
            decisions: state.decision_snapshots(window),
            operations: state.operation_snapshots(),
            history_capacity: self.0.config.history_capacity,
            history_error: state.history_error.clone(),
        }
    }

    pub(crate) fn window_snapshots(&self) -> Vec<WindowSnapshot> {
        self.0.state.borrow().window_snapshots()
    }

    pub(crate) fn tab_snapshots(&self) -> Vec<TabSnapshot> {
        self.0.state.borrow().tab_snapshots(None)
    }

    pub(crate) fn decision_snapshots(&self) -> Vec<DecisionSnapshot> {
        self.0.state.borrow().decision_snapshots(None)
    }

    pub(crate) fn operation_snapshots(&self) -> Vec<(OperationId, OperationState)> {
        self.0.state.borrow().operation_snapshots()
    }

    pub(crate) fn task_snapshots(
        &self,
        owner: Option<TaskOwner>,
    ) -> Vec<crate::tasks::TaskSnapshot> {
        self.0.tasks.task_snapshots(owner)
    }

    /// Returns retained operation state or an explicit expiration/unknown error.
    pub fn operation(&self, id: OperationId) -> Result<OperationState, OperationError> {
        let state = self.0.state.borrow();

        state
            .operations
            .get(&id)
            .cloned()
            .ok_or(if id.0 <= state.operation_sequence {
                OperationError::OperationExpired
            } else {
                OperationError::UnknownOperation
            })
    }

    /// Returns the shared local control client. Clones share request-key admission.
    pub fn client(&self) -> crate::control::RuntimeClient<R> {
        crate::control::RuntimeClient::new(self.clone(), self.0.control.clone())
    }

    pub(crate) fn has_window(&self) -> bool {
        self.0.has_window.get() && !self.0.state.borrow().windows.is_empty()
    }

    pub(crate) fn submit_command(
        &self,
        name: &str,
        token: CancellationToken,
        future: LocalBoxFuture<'static, Result<serde_json::Value, crate::control::CommandError>>,
    ) -> Operation<serde_json::Value> {
        let (sender, receiver) = oneshot::channel();

        let id = {
            let mut state = self.0.state.borrow_mut();

            state.operation_sequence += 1;

            OperationId(state.operation_sequence)
        };

        let handle = Operation {
            id,
            result: async move {
                receiver
                    .await
                    .unwrap_or(Err(OperationError::RuntimeShutdown))
            }
            .boxed_local()
            .shared(),
        };

        let admission = if self.0.shutdown.get() {
            Err(OperationError::RuntimeShutdown)
        } else if matches!(
            self.0.quit.borrow().phase,
            crate::quit::QuitPhase::Stopping | crate::quit::QuitPhase::Ready
        ) {
            Err(OperationError::Quitting)
        } else if self.0.reload.borrow().request.is_some() {
            Err(OperationError::Reloading)
        } else if self.0.state.borrow().tokens.len() >= self.0.config.pending_operations {
            Err(OperationError::QueueFull)
        } else {
            Ok(())
        };

        if let Err(error) = admission {
            let result = Err(error);

            self.0.finish::<serde_json::Value>(id, &result);
            let _ = sender.send(result);

            return handle;
        }

        self.0.state.borrow_mut().tokens.insert(id, token.clone());
        self.0.status(id, OperationState::Queued);
        self.0.event(id, "submitted");
        let core = Rc::downgrade(&self.0);
        let cancellation = token.clone();
        let future = future.instrument(tracing::info_span!("command.execute", command_id = name, operation_id = %id, runtime_id = %self.0.runtime));

        let job = async move {
            let Some(core) = core.upgrade() else {
                return;
            };

            core.status(id, OperationState::Running);
            let result = if core.shutdown.get() {
                Err(OperationError::RuntimeShutdown)
            } else if cancellation.is_cancelled() {
                Ok(Outcome::Cancelled(CancelReason::Requested))
            } else {
                match select(Box::pin(cancellation.cancelled()), Box::pin(future)).await {
                    Either::Right((result, _)) => result
                        .map(Outcome::Committed)
                        .map_err(OperationError::Command),
                    Either::Left(_) if core.shutdown.get() => Err(OperationError::RuntimeShutdown),
                    Either::Left(_) => Ok(Outcome::Cancelled(CancelReason::Requested)),
                }
            };

            core.finish(id, &result);
            let _ = sender.send(result);
        }
        .boxed_local();

        // Domain handlers manage their own model transactions and may await navigation.
        // Giving them a router lane could deadlock a handler awaiting that same lane.
        self.0.state.borrow_mut().queue.push_back(Job {
            lanes: Vec::new(),
            token,
            future: job,
        });
        self.0.work.notify(usize::MAX);

        handle
    }

    pub(crate) fn operation_result(&self, id: OperationId) -> Option<serde_json::Value> {
        self.0.state.borrow().results.get(&id).cloned()
    }

    pub(crate) fn frame_records(
        &self,
        store: &crate::inspection::Store,
        after: u64,
    ) -> crate::inspection::Frames {
        store.frames(after, &self.0.collector)
    }

    pub(crate) fn http_body(&self, request_id: u64, response: bool) -> serde_json::Value {
        #[cfg(feature = "devtools")]
        {
            serde_json::to_value(self.0.collector.http_body(request_id, response))
                .unwrap_or_default()
        }
        #[cfg(not(feature = "devtools"))]
        {
            let _ = (request_id, response);
            serde_json::Value::Null
        }
    }

    pub(crate) fn diagnostics(
        &self,
        after: u64,
        kind: Option<&str>,
    ) -> crate::diagnostics::Snapshot {
        self.0.collector.snapshot_after(after, kind)
    }

    pub(crate) fn cancel_task(&self, id: crate::tasks::TaskId) -> Result<bool, TaskError> {
        self.0.tasks.cancel(id)
    }

    pub(crate) fn is_shutdown(&self) -> bool {
        self.0.shutdown.get()
    }

    /// Finds a retained entry handle by identity, including non-current history entries.
    pub fn entry(&self, id: EntryId) -> Result<EntryHandle<R>, OperationError> {
        let state = self.0.state.borrow();

        let entry = state
            .tabs
            .iter()
            .flat_map(|tab| &tab.entries)
            .find(|entry| entry.id == id)
            .ok_or(OperationError::ClosedTarget)?;

        Ok(EntryHandle {
            core: Rc::downgrade(&self.0),
            entry: Rc::downgrade(entry),
        })
    }

    pub(crate) fn revision(&self) -> u64 {
        self.0.state.borrow().revision + self.0.tasks.revision()
    }

    /// Waits for a newer owned snapshot revision without a snapshot/subscription gap.
    pub async fn changed(&self, revision: u64) -> u64 {
        loop {
            let listener = self.0.event.listen();
            let work_revision = self.0.tasks.revision();
            let current = self.0.state.borrow().revision + work_revision;

            if current != revision {
                return current;
            }

            let _ = select(
                Box::pin(listener),
                Box::pin(self.0.tasks.changed(work_revision)),
            )
            .await;
        }
    }

    /// Requests cancellation. Returns false when the operation already finished.
    pub fn cancel(&self, id: OperationId) -> Result<bool, OperationError> {
        if matches!(
            self.operation(id)?,
            OperationState::Committed
                | OperationState::Blocked(_)
                | OperationState::Cancelled(_)
                | OperationState::Failed(_)
        ) {
            return Ok(false);
        }

        let token = self.0.state.borrow().tokens.get(&id).cloned();

        if let Some(token) = token {
            token.cancel();
            self.0.work.notify(usize::MAX);

            return Ok(true);
        }

        Ok(false)
    }

    /// Stops admission and cancels pending operations. Host termination is not a guarded quit.
    pub fn shutdown(&self) {
        self.0.shutdown.set(true);
        self.0.tasks.shutdown();
        #[cfg(feature = "gpui")]
        self.0.host.borrow_mut().clear();

        for token in self.0.state.borrow().tokens.values() {
            token.cancel();
        }

        for tab in &self.0.state.borrow().tabs {
            tab.scope.close();

            for entry in &tab.entries {
                entry.dispose();
            }
        }

        self.0.work.notify(usize::MAX);
    }

    /// Drives eagerly submitted jobs. Conflicting tab jobs run in order; other tabs continue.
    /// The host owns this future and must keep polling it for the runtime's lifetime.
    pub async fn run(&self) -> Result<(), OperationError> {
        if self.0.driving.replace(true) {
            return Err(OperationError::AlreadyDriving);
        }

        let _lifetime = DriverLifetime(Rc::downgrade(&self.0));
        let (result, ()) = futures::join!(self.drive_operations(), self.0.tasks.run());

        result
    }

    async fn drive_operations(&self) -> Result<(), OperationError> {
        let mut running = FuturesUnordered::new();
        let mut busy = BTreeSet::new();

        loop {
            let listener = self.0.work.listen();

            let empty = {
                let mut state = self.0.state.borrow_mut();
                let mut waiting = VecDeque::new();
                let mut blocked = BTreeSet::new();

                while let Some(mut job) = state.queue.pop_front() {
                    if job.token.is_cancelled() {
                        job.lanes.clear();
                    }

                    if job
                        .lanes
                        .iter()
                        .any(|lane| busy.contains(lane) || blocked.contains(lane))
                    {
                        blocked.extend(job.lanes.iter().copied());
                        waiting.push_back(job);
                    } else {
                        busy.extend(job.lanes.iter().copied());
                        running.push(
                            async move {
                                job.future.await;

                                job.lanes
                            }
                            .boxed_local(),
                        );
                    }
                }

                state.queue = waiting;

                state.tokens.is_empty()
            };

            if self.0.shutdown.get() && empty {
                return Ok(());
            }

            if running.is_empty() {
                listener.await;
            } else if let Either::Left((Some(lanes), _)) =
                select(Box::pin(running.next()), Box::pin(listener)).await
            {
                for lane in lanes {
                    busy.remove(&lane);
                }
            }
        }
    }

    /// Application-owned tasks survive internal tab close; their origin is retained.
    pub fn app(&self) -> AppHandle {
        let mut origin = self.0.tasks.origin();

        origin.window_id = self.0.initial_window.get();

        self.0.tasks.app(origin)
    }

    /// Supplies an actual host visibility signal, independently of keyboard focus.
    /// Unknown assumes visible while exposing that visibility is unmeasured.
    pub fn set_host_visibility(&self, visibility: HostVisibility) {
        let _ = self.set_window_visibility(self.0.initial_window.get(), visibility);
    }

    #[cfg(feature = "gpui")]
    pub(crate) fn observe_host(&self, window: &gpui::Window, cx: &gpui::App) {
        self.observe_host_in(self.0.initial_window.get(), window, cx);
    }

    #[cfg(feature = "gpui")]
    pub(crate) fn observe_host_in(&self, id: WindowId, window: &gpui::Window, cx: &gpui::App) {
        self.0.has_window.set(true);
        let core = Rc::downgrade(&self.0);
        let shutdown = core.clone();
        let unload = core.clone();

        #[cfg(not(target_family = "wasm"))]
        {
            let closing = core.clone();

            window.on_window_should_close(cx, move |_, _| {
                let Some(core) = closing.upgrade() else {
                    return true;
                };

                if core.quit.borrow().phase == crate::quit::QuitPhase::Ready {
                    return true;
                }

                if core.window_host.borrow().is_none() {
                    drop(Router(core).quit(crate::quit::QuitOptions::default()));

                    return false;
                }

                let opening = core
                    .state
                    .borrow()
                    .windows
                    .get(&id)
                    .and_then(|window| window.opening);

                if let Some(operation) = opening {
                    let _ = Router(core).cancel(operation);
                } else if !core.state.borrow().windows.contains_key(&id) {
                    return true;
                } else {
                    drop(Router(core).close_window(id));
                }

                false
            });
        }

        let observer = crate::tasks::host::HostObserver::attach(
            move |visibility| {
                if let Some(core) = core.upgrade() {
                    let _ = Router(core).set_window_visibility(id, visibility);
                }
            },
            move || {
                if let Some(core) = shutdown.upgrade() {
                    Router(core).shutdown();
                }
            },
            move || {
                let Some(core) = unload.upgrade() else {
                    return false;
                };

                #[cfg(target_family = "wasm")]
                if core.page_reload.replace(false) {
                    return false;
                }

                let router = Router(core);
                let snapshot = router.snapshot();

                snapshot.quit.phase != crate::quit::QuitPhase::Ready
                    && (snapshot.quit.phase.is_pending()
                        || snapshot
                            .tabs
                            .iter()
                            .flat_map(|tab| &tab.entries)
                            .any(|entry| entry.dirty)
                        || !snapshot.quit.pending_tasks.is_empty()
                        || !snapshot.quit.pending_operations.is_empty())
            },
            window,
            cx,
        );

        self.0.host.borrow_mut().insert(id, observer);
    }

    #[cfg(feature = "gpui")]
    pub(crate) fn resource(&self, window: WindowId) -> Option<Resource> {
        let state = self.0.state.borrow();

        let entry = state
            .tab(state.windows.get(&window)?.selected?)
            .ok()?
            .current();

        entry.resource.borrow().clone()
    }
}

struct DriverLifetime<R: Route>(Weak<Core<R>>);

impl<R: Route> Drop for DriverLifetime<R> {
    fn drop(&mut self) {
        if let Some(core) = self.0.upgrade() {
            Router(core).shutdown();
        }
    }
}

/// A weak, local command proxy. Concrete contextual targets are resolved on submission.
#[derive(Clone)]
pub struct Navigator<R: Route> {
    core: Weak<Core<R>>,
    window: WindowId,
    tab: Option<TabId>,
    origin_entry: Option<EntryId>,
    origin_scope: Weak<Scope>,
}

impl<R: Route> Navigator<R> {
    /// Dispatches typed application arguments through the same operation as the CLI.
    /// The page keeps only its weak navigator, avoiding a retained-view ownership cycle.
    pub fn dispatch<C: crate::control::ApplicationCommand>(
        &self,
        command: C,
        key: String,
    ) -> Result<crate::control::Reply, crate::control::ControlError> {
        Router(self.control_core()?).client().dispatch(command, key)
    }

    /// Application work with this navigator's original causal tab and entry IDs.
    pub fn app(&self) -> AppHandle {
        if let Some(core) = self.core.upgrade() {
            let mut origin = core.tasks.origin();

            origin.window_id = self.window;
            origin.tab_id = self.tab;
            origin.entry_id = self.origin_entry;

            core.tasks.app(origin)
        } else {
            AppHandle(TaskScope::unavailable())
        }
    }

    /// Eagerly opens a destination with default activation and tab mode.
    pub fn open(&self, destination: R, target: OpenTarget) -> Operation<Opened> {
        self.open_with(
            destination,
            OpenOptions {
                target,
                ..OpenOptions::new_tab()
            },
        )
    }

    /// Eagerly submits explicit creation options; new tabs never silently deduplicate.
    pub fn open_with(&self, destination: R, options: OpenOptions) -> Operation<Opened> {
        let owner = if options.target == OpenTarget::NewWindow {
            self.opening_core()
        } else {
            self.core()
        };

        let action = owner.and_then(|core| {
            let tab = if options.target == OpenTarget::CurrentTab {
                Some(
                    self.tab
                        .or_else(|| {
                            core.state
                                .borrow()
                                .windows
                                .get(&self.window)
                                .and_then(|window| window.selected)
                        })
                        .ok_or(OperationError::NoCurrentTab)?,
                )
            } else {
                None
            };

            if options.target == OpenTarget::CurrentTab && options.mode.is_some() {
                return Err(OperationError::InvalidOptions);
            }

            if options.target == OpenTarget::NewWindow && core.window_host.borrow().is_none() {
                return Err(OperationError::UnsupportedCapability(options.target));
            }

            Ok(Action::Open(destination, options, tab))
        });

        self.submit(action, |result| result.opened)
    }

    /// Targets an exact tab without semantic reuse.
    pub fn tab(&self, tab_id: TabId) -> TabHandle<R> {
        TabHandle {
            navigator: if self.tab == Some(tab_id) {
                self.clone()
            } else {
                self.core
                    .upgrade()
                    .and_then(|core| {
                        core.state
                            .borrow()
                            .tab(tab_id)
                            .ok()
                            .map(|tab| self.for_window(tab.window))
                    })
                    .unwrap_or_else(|| self.clone())
            },
            id: tab_id,
        }
    }

    /// Explicitly targets a host and clears the originating-tab binding.
    pub fn for_window(&self, window_id: WindowId) -> Self {
        Self {
            window: window_id,
            tab: None,
            origin_entry: None,
            origin_scope: Weak::new(),
            ..self.clone()
        }
    }

    /// Reuses the most recently selected matching current entry, or creates one tab.
    pub fn reveal(&self, destination: R, scope: RevealScope) -> Operation<Revealed> {
        self.submit(Ok(Action::Reveal(destination, scope)), |result| Revealed {
            opened: result.opened,
            reused: result.reused,
        })
    }

    /// Restores a browser checkpoint without pushing another browser visit.
    pub fn restore(&self, checkpoint: BrowserCheckpoint) -> Operation<Revealed> {
        let action = self.core().and_then(|core| {
            if checkpoint.version != 1 {
                return Err(RouteDecodeError::InvalidUri(
                    "unsupported browser checkpoint version".into(),
                )
                .into());
            }

            let destination = R::decode(&checkpoint.destination)?;
            let mut state = core.state.borrow_mut();

            state.advance_browser();
            let epoch = state.browser_epoch;

            Ok(Action::Restore(destination, Some(checkpoint), epoch))
        });

        if let Err(error) = &action {
            self.reconcile_history(error.to_string());
        }

        self.submit(action, |result| Revealed {
            opened: result.opened,
            reused: result.reused,
        })
    }

    /// Restores a decoded deep link without adding a browser visit or destroying other tabs.
    pub fn restore_destination(&self, destination: R) -> Operation<Revealed> {
        let action = self.core().map(|core| {
            let mut state = core.state.borrow_mut();

            state.advance_browser();

            Action::Restore(destination, None, state.browser_epoch)
        });

        self.submit(action, |result| Revealed {
            opened: result.opened,
            reused: result.reused,
        })
    }

    /// Resolves one currently valid decision. Unknown choices leave the decision pending.
    pub fn respond(&self, id: DecisionId, choice: &str) -> Result<(), DecisionError> {
        let core = self.control_core().map_err(|_| DecisionError::Obsolete)?;
        let mut state = core.state.borrow_mut();
        let decision = state.decisions.get(&id).ok_or(DecisionError::Obsolete)?;

        if decision
            .entry
            .upgrade()
            .is_none_or(|entry| !entry.alive.get() || entry.revision.get() != decision.revision)
        {
            state.decisions.remove(&id);

            return Err(DecisionError::Obsolete);
        }

        let choice = decision
            .choices
            .iter()
            .find(|candidate| candidate.descriptor.id == choice)
            .cloned()
            .ok_or(DecisionError::UnknownChoice)?;

        let decision = state.decisions.remove(&id).ok_or(DecisionError::Obsolete)?;

        drop(state);
        let result = decision
            .sender
            .send(Some(choice))
            .map_err(|_| DecisionError::Obsolete);

        core.notify();

        result
    }

    pub(crate) fn reconcile_history(&self, error: String) {
        if let Ok(core) = self.core() {
            core.state.borrow_mut().advance_browser();
            core.emit_history(true);
            core.state.borrow_mut().history_error = Some(error);
            core.notify();
        }
    }

    fn opening_core(&self) -> Result<Rc<Core<R>>, OperationError> {
        if self.tab.is_some() {
            return self.core();
        }

        let core = self.control_core()?;

        if core.quit.borrow().phase.blocks_navigation() {
            return Err(OperationError::Quitting);
        }

        Ok(core)
    }

    fn core(&self) -> Result<Rc<Core<R>>, OperationError> {
        let core = self.control_core()?;

        let state = core.state.borrow();

        let window = state
            .windows
            .get(&self.window)
            .ok_or(OperationError::WindowUnavailable)?;

        if window.opening.is_some() {
            return Err(OperationError::WindowUnavailable);
        }

        if window.closing.is_some() {
            return Err(OperationError::WindowClosing);
        }

        drop(state);

        if core.quit.borrow().phase.blocks_navigation() {
            return Err(OperationError::Quitting);
        }

        Ok(core)
    }

    fn control_core(&self) -> Result<Rc<Core<R>>, OperationError> {
        let core = self.core.upgrade().ok_or(OperationError::RuntimeShutdown)?;

        if core.shutdown.get() {
            return Err(OperationError::RuntimeShutdown);
        }

        if core.reload.borrow().request.is_some()
            || core.reload.borrow().phase == crate::reload::ReloadPhase::Restoring
        {
            return Err(OperationError::Reloading);
        }

        Ok(core)
    }

    fn submit<T: Clone + serde::Serialize + 'static>(
        &self,
        action: Result<Action<R>, OperationError>,
        map: impl FnOnce(Commit) -> T + 'static,
    ) -> Operation<T> {
        let (sender, receiver) = oneshot::channel();
        let core = self.core.upgrade();

        let id = core
            .as_ref()
            .map(|core| {
                let mut state = core.state.borrow_mut();

                state.operation_sequence += 1;

                OperationId(state.operation_sequence)
            })
            .unwrap_or(OperationId(0));

        let result = async move {
            receiver
                .await
                .unwrap_or(Err(OperationError::RuntimeShutdown))
        }
        .boxed_local()
        .shared();

        let handle = Operation { id, result };

        let Some(core) = core else {
            let _ = sender.send(Err(OperationError::RuntimeShutdown));

            return handle;
        };

        let owner = if matches!(&action, Ok(Action::Open(_, options, _)) if options.target == OpenTarget::NewWindow)
        {
            self.opening_core()
        } else {
            self.core()
        };

        let action = owner.and(action);

        let action = match action {
            Ok(action) if core.state.borrow().tokens.len() < core.config.pending_operations => {
                action
            }
            Ok(action) => {
                let result = Err(OperationError::QueueFull);

                core.finish::<T>(id, &result);

                if matches!(action, Action::Restore(..)) {
                    core.emit_history(true);
                }

                let _ = sender.send(result);

                return handle;
            }
            Err(error) => {
                let result = Err(error);

                core.finish::<T>(id, &result);
                let _ = sender.send(result);

                return handle;
            }
        };

        let window = self.window;
        let lanes = action.lanes(&core, window);
        let token = CancellationToken::new();

        core.state.borrow_mut().tokens.insert(id, token.clone());
        core.status(id, OperationState::Queued);
        core.event(id, "submitted");
        let owned = Rc::downgrade(&core);
        let queued_token = token.clone();

        let job = async move {
            let Some(owned) = owned.upgrade() else {
                let _ = sender.send(Err(OperationError::RuntimeShutdown));

                return;
            };

            owned.status(id, OperationState::Running);
            let result = if owned.shutdown.get() {
                Err(OperationError::RuntimeShutdown)
            } else if token.is_cancelled() {
                Ok(Outcome::Cancelled(CancelReason::Requested))
            } else {
                let execution = async {
                    let execution = execute(&owned, id, &action, window)
                        .instrument(tracing::info_span!("router.operation", operation_id = %id));

                    if let Action::Restore(_, _, epoch) = &action {
                        match select(Box::pin(execution), Box::pin(superseded(&owned, *epoch)))
                            .await
                        {
                            Either::Left((result, _)) => result,
                            Either::Right(_) => Ok(Outcome::Cancelled(CancelReason::Superseded)),
                        }
                    } else {
                        execution.await
                    }
                };

                match select(Box::pin(token.cancelled()), Box::pin(execution)).await {
                    Either::Right((result, _)) => result,
                    Either::Left(_) if owned.shutdown.get() => Err(OperationError::RuntimeShutdown),
                    Either::Left(_) => Ok(Outcome::Cancelled(CancelReason::Requested)),
                }
            };

            if matches!(action, Action::Restore(..))
                && matches!(result, Ok(Outcome::Cancelled(CancelReason::Requested)))
                && action.check(&owned).is_ok()
            {
                owned.emit_history(true);
            }

            let result = result.map(|outcome| match outcome {
                Outcome::Committed(value) => Outcome::Committed(map(value)),
                Outcome::Cancelled(reason) => Outcome::Cancelled(reason),
                Outcome::Blocked(reason) => Outcome::Blocked(reason),
            });

            owned.finish(id, &result);
            let _ = sender.send(result);
        }
        .boxed_local();

        core.state.borrow_mut().queue.push_back(Job {
            lanes,
            token: queued_token,
            future: job,
        });
        core.work.notify(usize::MAX);

        handle
    }
}

/// A command handle to a concrete tab; it never retargets after an asynchronous guard.
#[derive(Clone)]
pub struct TabHandle<R: Route> {
    navigator: Navigator<R>,
    id: TabId,
}

impl<R: Route> TabHandle<R> {
    /// Tab-owned work continues across hiding and navigation; close cancels it.
    pub fn tasks(&self) -> TaskScope {
        let Some(core) = self.navigator.core.upgrade() else {
            return TaskScope::unavailable();
        };

        let state = core.state.borrow();

        let scope = state
            .tab(self.id)
            .ok()
            .map(|tab| tab.scope.clone())
            .or_else(|| {
                (self.navigator.tab == Some(self.id))
                    .then(|| self.navigator.origin_scope.upgrade())
                    .flatten()
            });

        let Some(scope) = scope else {
            return TaskScope::unavailable();
        };

        let mut origin = core.tasks.origin();

        origin.window_id = state
            .tab(self.id)
            .map_or(self.navigator.window, |tab| tab.window);
        origin.tab_id = Some(self.id);
        origin.entry_id = self.navigator.origin_entry;

        scope.handle(origin)
    }

    /// Pushes a new retained instance, guarding disposal of any forward branch.
    pub fn navigate(&self, destination: R) -> Operation<Entered> {
        self.navigator.submit(
            Ok(Action::Navigate(self.id, destination, false)),
            |result| result.opened,
        )
    }

    /// Replaces the current instance after its disposal guards allow it.
    pub fn replace(&self, destination: R) -> Operation<Entered> {
        self.navigator
            .submit(Ok(Action::Navigate(self.id, destination, true)), |result| {
                result.opened
            })
    }

    /// Moves to a retained preceding entry without rebuilding its view.
    pub fn back(&self) -> Operation<Entered> {
        self.navigator
            .submit(Ok(Action::Move(self.id, -1)), |result| result.opened)
    }

    /// Moves to a retained following entry without rebuilding its view.
    pub fn forward(&self) -> Operation<Entered> {
        self.navigator
            .submit(Ok(Action::Move(self.id, 1)), |result| result.opened)
    }

    /// Changes selection without adding a per-tab history entry.
    pub fn select(&self) -> Operation<Selected> {
        self.navigator
            .submit(Ok(Action::Select(self.id)), |result| result.opened)
    }

    /// Guards and disposes every retained entry in this tab.
    pub fn close(&self) -> Operation<Closed> {
        self.navigator
            .submit(Ok(Action::Close(self.id)), |result| Closed {
                tab_id: result.opened.tab_id,
            })
    }

    /// Explicitly changes the stored work policy of an existing tab.
    pub fn set_mode(&self, mode: TabMode) -> Operation<ModeChanged> {
        self.navigator
            .submit(Ok(Action::Mode(self.id, mode)), move |result| ModeChanged {
                tab_id: result.opened.tab_id,
                mode,
            })
    }
}

/// A weak screen-lifetime handle usable by guards and retained view state.
#[derive(Clone)]
pub struct EntryHandle<R: Route> {
    core: Weak<Core<R>>,
    entry: Weak<Entry<R>>,
}

impl<R: Route> EntryHandle<R> {
    fn tasks(&self) -> Result<TaskScope, TaskError> {
        let entry = self.live().map_err(|_| TaskError::OwnerDisposed)?;
        let core = self.core.upgrade().ok_or(TaskError::OwnerDisposed)?;
        let mut origin = core.tasks.origin();

        origin.tab_id = Some(entry.tab);
        origin.window_id = entry.window;
        origin.entry_id = Some(entry.id);

        Ok(entry.scope.handle(origin))
    }

    /// Registers a fresh-future factory following this entry's Foreground/Live eligibility.
    pub fn activity<F, Fut>(
        &self,
        key: impl Into<String>,
        factory: F,
    ) -> Result<TaskHandle, TaskError>
    where
        F: Fn(ActivityRun) -> Fut + 'static,
        Fut: Future<Output = Result<(), TaskError>> + 'static,
    {
        self.tasks()?
            .activity(key.into(), TaskKind::Activity, factory)
    }

    /// Registers a factory requiring presentation and host visibility in either tab mode.
    pub fn while_visible<F, Fut>(
        &self,
        key: impl Into<String>,
        factory: F,
    ) -> Result<TaskHandle, TaskError>
    where
        F: Fn(ActivityRun) -> Fut + 'static,
        Fut: Future<Output = Result<(), TaskError>> + 'static,
    {
        self.tasks()?
            .activity(key.into(), TaskKind::WhileVisible, factory)
    }

    /// Installs a typed handler for checked publications into this retained entry.
    pub fn on_event<T: 'static>(
        &self,
        handler: impl Fn(T) -> Result<(), TaskError> + 'static,
    ) -> Result<(), TaskError> {
        self.tasks()?.on_event(handler)
    }

    /// Returns the host owning this retained screen, while its lifetime remains valid.
    pub fn window_id(&self) -> Result<WindowId, OperationError> {
        Ok(self.live()?.window)
    }

    /// Returns the concrete tab owning this retained screen.
    pub fn tab_id(&self) -> Result<TabId, OperationError> {
        Ok(self.live()?.tab)
    }

    /// Returns the retained screen identity while it remains alive.
    pub fn id(&self) -> Result<EntryId, OperationError> {
        Ok(self.live()?.id)
    }

    /// Returns its state revision for application save/revalidation policies.
    pub fn revision(&self) -> Result<u64, OperationError> {
        Ok(self.live()?.revision.get())
    }

    /// Returns its current dirty flag.
    pub fn is_dirty(&self) -> Result<bool, OperationError> {
        Ok(self.live()?.dirty.get())
    }

    /// Marks a new state revision, even if dirty was already true, and expires old decisions.
    pub fn set_dirty(&self, dirty: bool) -> Result<(), OperationError> {
        if self.core.upgrade().is_some_and(|core| {
            matches!(
                core.quit.borrow().phase,
                crate::quit::QuitPhase::Stopping | crate::quit::QuitPhase::Ready
            )
        }) {
            return Err(OperationError::Quitting);
        }

        let entry = self.live()?;
        let core = self.core.upgrade().ok_or(OperationError::RuntimeShutdown)?;

        if core.reload.borrow().request.is_some() {
            return Err(OperationError::Reloading);
        }

        entry.dirty.set(dirty);
        entry.revision.set(entry.revision.get() + 1);
        let core = self.core.upgrade().ok_or(OperationError::RuntimeShutdown)?;

        core.state.borrow_mut().invalidate(Some(entry.id));
        core.notify();

        Ok(())
    }

    /// Registers a disposal guard. Without custom guards, dirty entries ask discard/cancel.
    pub fn on_dispose<F, Fut>(&self, guard: F) -> Result<(), OperationError>
    where
        F: Fn(GuardContext<R>) -> Fut + 'static,
        Fut: Future<Output = GuardResult> + 'static,
    {
        let entry = self.live()?;

        self.set_dirty(entry.dirty.get())?;
        entry
            .guards
            .borrow_mut()
            .push(Rc::new(move |context| Box::pin(guard(context))));

        Ok(())
    }

    /// Returns this instance's cancellation signal; disposal requests cancellation.
    pub fn cancellation(&self) -> Result<CancellationToken, OperationError> {
        Ok(self.live()?.cancellation.clone())
    }

    fn live(&self) -> Result<Rc<Entry<R>>, OperationError> {
        if self.core.upgrade().is_none_or(|core| core.shutdown.get()) {
            return Err(OperationError::RuntimeShutdown);
        }

        self.entry
            .upgrade()
            .filter(|entry| entry.alive.get())
            .ok_or(OperationError::ClosedTarget)
    }
}

enum Action<R: Route> {
    Quit,
    CloseWindow,
    Open(R, OpenOptions, Option<TabId>),
    Navigate(TabId, R, bool),
    Move(TabId, i8),
    Select(TabId),
    Close(TabId),
    Mode(TabId, TabMode),
    Reveal(R, RevealScope),
    Restore(R, Option<BrowserCheckpoint>, u64),
}

impl<R: Route> Action<R> {
    fn lanes(&self, core: &Core<R>, window: WindowId) -> Vec<Lane> {
        match self {
            Self::Open(_, _, Some(tab))
            | Self::Navigate(tab, ..)
            | Self::Move(tab, _)
            | Self::Select(tab)
            | Self::Close(tab)
            | Self::Mode(tab, _) => vec![Lane::Tab(*tab)],
            Self::Restore(_, Some(checkpoint), _) if checkpoint.runtime_id == core.runtime => {
                vec![Lane::Workspace, Lane::Tab(checkpoint.tab_id)]
            }
            Self::Open(_, options, _) if options.target == OpenTarget::NewWindow => {
                vec![Lane::Workspace]
            }
            Self::Reveal(_, RevealScope::Application) => vec![Lane::Workspace],
            _ => vec![Lane::Window(window)],
        }
    }

    fn check(&self, core: &Core<R>) -> Result<(), Stop> {
        if core.shutdown.get() {
            return Err(OperationError::RuntimeShutdown.into());
        }

        if core.reload.borrow().request.is_some() {
            return Err(OperationError::Reloading.into());
        }

        if let Self::Restore(_, _, epoch) = self
            && *epoch != core.state.borrow().browser_epoch
        {
            return Err(Stop::Cancelled(CancelReason::Superseded));
        }

        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Transition {
    New(Activation, TabMode),
    Push,
    Replace,
    Cursor(usize),
    Select,
    Close,
    Mode(TabMode),
}

struct Plan<R: Route> {
    entry: Rc<Entry<R>>,
    transition: Transition,
    expected: Option<EntryId>,
    disposed: Vec<(Rc<Entry<R>>, DisposeReason)>,
    reused: bool,
}

struct Commit {
    opened: Opened,
    reused: bool,
}

enum Stop {
    Error(OperationError),
    Blocked(BlockReason),
    Cancelled(CancelReason),
}

async fn superseded<R: Route>(core: &Core<R>, epoch: u64) {
    loop {
        let listener = core.event.listen();

        if core.state.borrow().browser_epoch != epoch {
            return;
        }

        listener.await;
    }
}

impl From<OperationError> for Stop {
    fn from(error: OperationError) -> Self {
        Self::Error(error)
    }
}

impl From<RouteDecodeError> for Stop {
    fn from(error: RouteDecodeError) -> Self {
        Self::Error(error.into())
    }
}

fn fresh<R: Route>(
    core: &Core<R>,
    state: &mut State<R>,
    tab: TabId,
    window: WindowId,
    destination: R,
) -> Result<Rc<Entry<R>>, Stop> {
    let destination = R::decode(&destination.encode())?;

    destination.to_uri()?;

    if core.config.history_capacity == Some(0) {
        return Err(Stop::Blocked(BlockReason::HistoryCapacity));
    }

    let id = EntryId(state.next());

    Ok(Rc::new(Entry {
        scope: core.tasks.scope(TaskOwner::Entry(id)),
        tab_scope: state
            .tab(tab)
            .map(|tab| tab.scope.clone())
            .unwrap_or_else(|_| core.tasks.scope(TaskOwner::Tab(tab))),
        id,
        tab,
        window,
        destination,
        revision: Cell::new(0),
        dirty: Cell::new(false),
        alive: Cell::new(true),
        guards: RefCell::new(Vec::new()),
        resource: RefCell::new(None),
        cancellation: CancellationToken::new(),
        reload_capture: RefCell::new(None),
        restored_state: RefCell::new(None),
        state_consumed: Cell::new(false),
    }))
}

fn plan<R: Route>(core: &Core<R>, action: &Action<R>, window: WindowId) -> Result<Plan<R>, Stop> {
    let mut state = core.state.borrow_mut();

    let new = |state: &mut State<R>, destination: R, options: OpenOptions| {
        let window = if options.target == OpenTarget::NewWindow {
            WindowId(state.next().max(2))
        } else {
            window
        };

        let tab = TabId(state.next());
        let entry = fresh(core, state, tab, window, destination)?;

        Ok(Plan {
            entry,
            transition: Transition::New(
                options.activation,
                options.mode.unwrap_or(core.config.default_mode),
            ),
            expected: None,
            disposed: Vec::new(),
            reused: false,
        })
    };

    match action {
        Action::Quit | Action::CloseWindow => Err(OperationError::InvalidOptions.into()),
        Action::Open(destination, options, None) => new(&mut state, destination.clone(), *options),
        Action::Open(destination, _, Some(tab)) | Action::Navigate(tab, destination, false) => {
            let current = state.tab(*tab)?;

            if core
                .config
                .history_capacity
                .is_some_and(|capacity| current.cursor + 2 > capacity)
            {
                return Err(Stop::Blocked(BlockReason::HistoryCapacity));
            }

            let expected = current.current().id;

            let disposed = current.entries[current.cursor + 1..]
                .iter()
                .map(|entry| (entry.clone(), DisposeReason::TruncateForwardHistory))
                .collect();

            let host = state.tab(*tab)?.window;
            let entry = fresh(core, &mut state, *tab, host, destination.clone())?;

            Ok(Plan {
                entry,
                transition: Transition::Push,
                expected: Some(expected),
                disposed,
                reused: false,
            })
        }
        Action::Navigate(tab, destination, true) => {
            let outgoing = state.tab(*tab)?.current();
            let host = state.tab(*tab)?.window;
            let entry = fresh(core, &mut state, *tab, host, destination.clone())?;

            Ok(Plan {
                entry,
                transition: Transition::Replace,
                expected: Some(outgoing.id),
                disposed: vec![(outgoing, DisposeReason::ReplaceEntry)],
                reused: false,
            })
        }
        Action::Move(tab, direction) => {
            let tab = state.tab(*tab)?;

            let cursor = tab
                .cursor
                .checked_add_signed(*direction as isize)
                .filter(|cursor| *cursor < tab.entries.len())
                .ok_or(Stop::Blocked(BlockReason::HistoryBoundary))?;

            Ok(Plan {
                entry: tab.entries[cursor].clone(),
                transition: Transition::Cursor(cursor),
                expected: Some(tab.current().id),
                disposed: Vec::new(),
                reused: true,
            })
        }
        Action::Select(tab) | Action::Close(tab) | Action::Mode(tab, _) => {
            let tab = state.tab(*tab)?;

            let transition = match action {
                Action::Close(_) => Transition::Close,
                Action::Mode(_, mode) => Transition::Mode(*mode),
                _ => Transition::Select,
            };

            let disposed = if matches!(transition, Transition::Close) {
                tab.entries
                    .iter()
                    .map(|entry| (entry.clone(), DisposeReason::CloseTab))
                    .collect()
            } else {
                Vec::new()
            };

            Ok(Plan {
                entry: tab.current(),
                transition,
                expected: Some(tab.current().id),
                disposed,
                reused: true,
            })
        }
        Action::Reveal(destination, _) | Action::Restore(destination, _, _) => {
            if let Action::Restore(_, Some(checkpoint), _) = action
                && checkpoint.runtime_id == core.runtime
                && let Ok(tab) = state.tab(checkpoint.tab_id)
                && let Some(cursor) = tab.entries.iter().position(|entry| {
                    entry.id == checkpoint.entry_id
                        && entry.destination.encode() == destination.encode()
                })
            {
                return Ok(Plan {
                    entry: tab.entries[cursor].clone(),
                    transition: Transition::Cursor(cursor),
                    expected: Some(tab.current().id),
                    disposed: Vec::new(),
                    reused: true,
                });
            }

            if let Some(entry) = state.matching(
                destination,
                match action {
                    Action::Reveal(_, RevealScope::Application) => None,
                    _ => Some(window),
                },
            ) {
                return Ok(Plan {
                    expected: Some(entry.id),
                    entry,
                    transition: Transition::Select,
                    disposed: Vec::new(),
                    reused: true,
                });
            }

            new(&mut state, destination.clone(), OpenOptions::new_tab())
        }
    }
}

async fn guard<R: Route>(
    core: &Core<R>,
    id: OperationId,
    action: &Action<R>,
    entry: &Rc<Entry<R>>,
    kind: GuardKind,
    origin_window: WindowId,
) -> Result<u64, Stop> {
    match select(
        Box::pin(evaluate_guard(core, id, action, entry, kind, origin_window)),
        Box::pin(entry.cancellation.cancelled()),
    )
    .await
    {
        Either::Left((result, _)) => result,
        Either::Right(_) => Err(OperationError::ClosedTarget.into()),
    }
}

async fn evaluate_guard<R: Route>(
    core: &Core<R>,
    id: OperationId,
    action: &Action<R>,
    entry: &Rc<Entry<R>>,
    kind: GuardKind,
    origin_window: WindowId,
) -> Result<u64, Stop> {
    'revalidate: loop {
        action.check(core)?;

        if !entry.alive.get() {
            return Err(OperationError::ClosedTarget.into());
        }

        let revision = entry.revision.get();

        let mut guards = match kind {
            GuardKind::Enter => core.guards.borrow().clone(),
            GuardKind::Dispose(_) => entry.guards.borrow().clone(),
        };

        let default_dirty =
            guards.is_empty() && matches!(kind, GuardKind::Dispose(_)) && entry.dirty.get();

        if default_dirty {
            guards.push(Rc::new(|_| {
                Box::pin(async {
                    GuardResult::Ask(DecisionRequest {
                        message: "Discard unsaved changes?".into(),
                        choices: vec![
                            DecisionChoice::proceed("discard", "Discard"),
                            DecisionChoice::cancel("cancel", "Cancel"),
                        ],
                    })
                })
            }));
        }

        for callback in guards {
            let context = GuardContext {
                window_id: entry.window,
                operation_id: id,
                tab_id: entry.tab,
                entry_id: entry.id,
                destination: entry.destination.clone(),
                dirty: entry.dirty.get(),
                revision,
                kind,
            };

            let result = callback(context)
                .instrument(tracing::info_span!("router.guard", entry_id = %entry.id))
                .await;

            action.check(core)?;

            if !entry.alive.get() {
                return Err(OperationError::ClosedTarget.into());
            }

            if revision != entry.revision.get() {
                continue 'revalidate;
            }

            match result {
                GuardResult::Allow => {}
                GuardResult::Deny(reason) => return Err(Stop::Blocked(BlockReason::Guard(reason))),
                GuardResult::Ask(request) => {
                    let mut choices = BTreeSet::new();

                    if request.choices.is_empty()
                        || request.choices.iter().any(|choice| {
                            choice.descriptor.id.is_empty()
                                || !choices.insert(choice.descriptor.id.clone())
                        })
                    {
                        return Err(OperationError::InvalidDecision(
                            "choices must have nonempty unique IDs".into(),
                        )
                        .into());
                    }

                    let (sender, receiver) = oneshot::channel();

                    let (decision, presentation_window) = {
                        let mut state = core.state.borrow_mut();
                        let decision = DecisionId(state.next());

                        let window_id = if state.windows.contains_key(&entry.window) {
                            entry.window
                        } else if state.windows.contains_key(&origin_window) {
                            origin_window
                        } else {
                            state
                                .windows
                                .keys()
                                .next()
                                .copied()
                                .unwrap_or(origin_window)
                        };

                        let snapshot = DecisionSnapshot {
                            window_id,
                            id: decision,
                            operation_id: id,
                            entry_id: entry.id,
                            message: request.message,
                            choices: request
                                .choices
                                .iter()
                                .map(|choice| choice.descriptor.clone())
                                .collect(),
                        };

                        let browser_epoch = match action {
                            Action::Restore(_, _, epoch) => Some(*epoch),
                            _ => None,
                        };

                        state.decisions.insert(
                            decision,
                            PendingDecision {
                                snapshot,
                                entry: Rc::downgrade(entry),
                                revision,
                                browser_epoch,
                                sender,
                                choices: request.choices,
                            },
                        );

                        (decision, window_id)
                    };

                    core.status(id, OperationState::AwaitingDecision(decision));
                    core.event(id, "awaiting_decision");
                    let host = core.window_host.borrow().clone();

                    if let Some(host) = host {
                        host.activate(presentation_window);
                    }

                    let choice = receiver
                        .await
                        .map_err(|_| OperationError::RuntimeShutdown)?;

                    core.status(id, OperationState::Running);
                    action.check(core)?;

                    if !entry.alive.get() {
                        return Err(OperationError::ClosedTarget.into());
                    }

                    let Some(choice) = choice else {
                        continue 'revalidate;
                    };

                    if choice.descriptor.action == ChoiceAction::Cancel {
                        return Err(Stop::Cancelled(CancelReason::Decision));
                    }

                    if revision != entry.revision.get() {
                        continue 'revalidate;
                    }

                    if let Some(prepare) = choice.prepare {
                        prepare()
                            .await
                            .map_err(|reason| Stop::Blocked(BlockReason::Guard(reason)))?;
                    }

                    action.check(core)?;

                    if revision != entry.revision.get() {
                        continue 'revalidate;
                    }
                }
            }
        }

        return Ok(revision);
    }
}

async fn transition<R: Route>(
    core: &Rc<Core<R>>,
    id: OperationId,
    action: &Action<R>,
    window: WindowId,
) -> Result<Commit, Stop> {
    'plan: loop {
        action.check(core)?;
        let plan = plan(core, action, window)?;
        let mut provisional = None;

        loop {
            let mut approvals = Vec::new();

            if !matches!(plan.transition, Transition::Close | Transition::Mode(_)) {
                approvals.push((
                    plan.entry.clone(),
                    guard(core, id, action, &plan.entry, GuardKind::Enter, window).await?,
                ));
            }

            for (entry, reason) in &plan.disposed {
                approvals.push((
                    entry.clone(),
                    guard(core, id, action, entry, GuardKind::Dispose(*reason), window).await?,
                ));
            }

            if provisional.is_none()
                && matches!(action, Action::Open(_, options, _) if options.target == OpenTarget::NewWindow)
            {
                provisional = Some(windows::ProvisionalWindow::new(
                    core.clone(),
                    plan.entry.window,
                    id,
                ));
            }

            if let Some(provisional) = &mut provisional {
                provisional
                    .open(match action {
                        Action::Open(_, options, _) => options.activation,
                        _ => Activation::Activate,
                    })
                    .await?;
            }

            if plan.entry.resource.borrow().is_none()
                && !matches!(plan.transition, Transition::Close | Transition::Mode(_))
            {
                let handle = EntryHandle {
                    core: Rc::downgrade(core),
                    entry: Rc::downgrade(&plan.entry),
                };

                let navigator = Navigator {
                    core: Rc::downgrade(core),
                    window: plan.entry.window,
                    tab: Some(plan.entry.tab),
                    origin_entry: Some(plan.entry.id),
                    origin_scope: Rc::downgrade(&plan.entry.tab_scope),
                };

                let resource = (core.mount)(plan.entry.destination.clone(), handle, navigator)
                    .instrument(tracing::info_span!("router.mount", entry_id = %plan.entry.id))
                    .await
                    .map_err(OperationError::Mount)?;

                *plan.entry.resource.borrow_mut() = Some(resource);
            }

            action.check(core)?;

            if approvals.iter().any(|(entry, _)| !entry.alive.get()) {
                return Err(OperationError::ClosedTarget.into());
            }

            if approvals
                .iter()
                .any(|(entry, revision)| entry.revision.get() != *revision)
            {
                continue;
            }

            let state = core.state.borrow();

            if let Some(expected) = plan.expected
                && state.tab(plan.entry.tab)?.current().id != expected
            {
                continue 'plan;
            }

            if let Action::Reveal(destination, _) = action
                && state
                    .matching(
                        destination,
                        match action {
                            Action::Reveal(_, RevealScope::Application) => None,
                            _ => Some(window),
                        },
                    )
                    .map(|entry| entry.id)
                    != plan.expected
            {
                continue 'plan;
            }

            if let Action::Restore(destination, _, _) = action
                && matches!(plan.transition, Transition::New(..) | Transition::Select)
                && state
                    .matching(
                        destination,
                        match action {
                            Action::Reveal(_, RevealScope::Application) => None,
                            _ => Some(window),
                        },
                    )
                    .map(|entry| entry.id)
                    != plan.expected
            {
                continue 'plan;
            }

            drop(state);

            // A guard or mount can cancel synchronously within this execution poll.
            if core
                .state
                .borrow()
                .tokens
                .get(&id)
                .is_some_and(CancellationToken::is_cancelled)
            {
                return Err(Stop::Cancelled(CancelReason::Requested));
            }

            commit(core, id, action, &plan)?;

            if let Some(provisional) = &mut provisional {
                provisional.committed = true;
            }

            core.event(id, "committed");

            return Ok(Commit {
                opened: Opened {
                    window_id: plan.entry.window,
                    tab_id: plan.entry.tab,
                    entry_id: plan.entry.id,
                },
                reused: plan.reused,
            });
        }
    }
}

fn commit<R: Route>(
    core: &Core<R>,
    operation_id: OperationId,
    action: &Action<R>,
    plan: &Plan<R>,
) -> Result<(), OperationError> {
    let mut state = core.state.borrow_mut();

    let window = plan.entry.window;

    let host = state
        .windows
        .get(&window)
        .ok_or(OperationError::WindowUnavailable)?;

    if host.closing.is_some() {
        return Err(OperationError::WindowClosing);
    }

    let before = state
        .windows
        .get(&window)
        .and_then(|host| host.selected)
        .and_then(|tab| state.tab(tab).ok().map(|tab| (tab.id, tab.current().id)));

    let tab = plan.entry.tab;

    for (entry, _) in &plan.disposed {
        entry.dispose();
        state.invalidate(Some(entry.id));
    }

    match plan.transition {
        Transition::New(activation, mode) => {
            state.tabs.push(Tab {
                scope: plan.entry.tab_scope.clone(),
                id: tab,
                window,
                entries: vec![plan.entry.clone()],
                cursor: 0,
                mode,
                selected_at: 0,
            });

            if let Some(host) = state.windows.get_mut(&window) {
                host.opening = None;
            }

            if activation == Activation::Activate
                || state
                    .windows
                    .get(&window)
                    .is_some_and(|host| host.selected.is_none())
            {
                state.select(tab);
            }
        }
        Transition::Push => {
            let tab = state.tab_mut(tab)?;

            tab.entries.truncate(tab.cursor + 1);
            tab.entries.push(plan.entry.clone());
            tab.cursor += 1;
        }
        Transition::Replace => {
            let tab = state.tab_mut(tab)?;

            tab.entries[tab.cursor] = plan.entry.clone();
        }
        Transition::Cursor(cursor) => {
            state.tab_mut(tab)?.cursor = cursor;

            if matches!(action, Action::Restore(..)) {
                state.select(tab);
            }
        }
        Transition::Select => state.select(tab),
        Transition::Mode(mode) => state.tab_mut(tab)?.mode = mode,
        Transition::Close => {
            let index = state
                .tabs
                .iter()
                .position(|candidate| candidate.id == tab)
                .ok_or(OperationError::ClosedTarget)?;

            state.tabs[index].scope.close();
            state.tabs.remove(index);

            if state
                .windows
                .get(&window)
                .is_some_and(|host| host.selected == Some(tab))
            {
                let replacement = state
                    .tabs
                    .iter()
                    .filter(|tab| tab.window == window)
                    .max_by_key(|tab| tab.selected_at)
                    .map(|tab| tab.id);

                state.windows.get_mut(&window).expect("tab host").selected = None;

                if let Some(selected) = replacement {
                    state.select(selected);
                }
            }
        }
    }

    let after = state
        .windows
        .get(&window)
        .and_then(|host| host.selected)
        .and_then(|tab| state.tab(tab).ok().map(|tab| (tab.id, tab.current().id)));

    let restore = matches!(action, Action::Restore(..));

    if !restore && before != after {
        state.advance_browser();
    }

    let opened = Opened {
        window_id: window,
        tab_id: plan.entry.tab,
        entry_id: plan.entry.id,
    };

    let result = match action {
        Action::Close(tab) => serde_json::to_value(Closed { tab_id: *tab }),
        Action::Mode(tab, mode) => serde_json::to_value(ModeChanged {
            tab_id: *tab,
            mode: *mode,
        }),
        Action::Reveal(..) | Action::Restore(..) => serde_json::to_value(Revealed {
            opened,
            reused: plan.reused,
        }),
        _ => serde_json::to_value(opened),
    }
    .expect("framework operation result is serializable");

    state.results.insert(operation_id, result);
    state
        .operations
        .insert(operation_id, OperationState::Committed);

    drop(state);
    {
        let state = core.state.borrow();

        for tab in &state.tabs {
            for entry in &tab.entries {
                let current = entry.id == tab.current().id;

                entry.scope.update(
                    current,
                    current
                        && state
                            .windows
                            .get(&tab.window)
                            .is_some_and(|host| host.selected == Some(tab.id)),
                    tab.mode == TabMode::Live,
                );
            }
        }
    }
    core.notify();

    if matches!(action, Action::Reveal(..) | Action::Select(_)) {
        let host = core.window_host.borrow().clone();

        if let Some(host) = host {
            host.activate(window);
        }
    }

    if window == core.initial_window.get() && (restore || before != after) {
        core.emit_history(
            restore || matches!(plan.transition, Transition::Replace | Transition::Close),
        );
    }

    Ok(())
}

async fn execute<R: Route>(
    core: &Rc<Core<R>>,
    id: OperationId,
    action: &Action<R>,
    window: WindowId,
) -> Result<Outcome<Commit>, OperationError> {
    let result = match transition(core, id, action, window).await {
        Ok(value) => Ok(Outcome::Committed(value)),
        Err(Stop::Blocked(reason)) => Ok(Outcome::Blocked(reason)),
        Err(Stop::Cancelled(reason)) => Ok(Outcome::Cancelled(reason)),
        Err(Stop::Error(error)) => Err(error),
    };

    if matches!(action, Action::Restore(..))
        && !matches!(
            result,
            Ok(Outcome::Committed(_)) | Ok(Outcome::Cancelled(CancelReason::Superseded))
        )
    {
        core.emit_history(true);
    }

    result
}

impl<R: Route> State<R> {
    fn window_snapshots(&self) -> Vec<WindowSnapshot> {
        self.windows
            .iter()
            .map(|(id, window)| WindowSnapshot {
                id: *id,
                selected: window.selected,
                closing: window.closing,
                visibility: window.visibility,
            })
            .collect()
    }

    fn tab_snapshots(&self, window: Option<WindowId>) -> Vec<TabSnapshot> {
        self.tabs
            .iter()
            .filter(|tab| window.is_none_or(|window| tab.window == window))
            .map(|tab| TabSnapshot {
                window_id: tab.window,
                id: tab.id,
                mode: tab.mode,
                cursor: tab.cursor,
                entries: tab
                    .entries
                    .iter()
                    .map(|entry| EntrySnapshot {
                        id: entry.id,
                        destination: entry.destination.encode(),
                        dirty: entry.dirty.get(),
                        revision: entry.revision.get(),
                    })
                    .collect(),
            })
            .collect()
    }

    fn decision_snapshots(&self, window: Option<WindowId>) -> Vec<DecisionSnapshot> {
        self.decisions
            .values()
            .filter(|decision| window.is_none_or(|window| decision.snapshot.window_id == window))
            .map(|decision| decision.snapshot.clone())
            .collect()
    }

    fn operation_snapshots(&self) -> Vec<(OperationId, OperationState)> {
        self.operations
            .iter()
            .map(|(id, status)| (*id, status.clone()))
            .collect()
    }
}
