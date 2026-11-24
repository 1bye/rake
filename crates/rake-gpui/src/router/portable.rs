//! Inherent routing methods forwarded to the shared scheduler.

use std::future::Future;
#[cfg(feature = "updates")]
use std::rc::Rc;

use super::{
    BrowserCheckpoint, DecisionError, DecisionId, EntryHandle, EntryId, GuardContext, GuardReason,
    GuardResult, HistoryChange, NavigationKey, NavigationStatus, Navigator, OpenOptions,
    OpenTarget, Opened, Operation, OperationError, OperationId, OperationState, QuitGuardContext,
    RevealScope, Revealed, Route, Router, RouterSnapshot, RuntimeId, TabHandle, TabId,
    WindowClosed, WindowHost, WindowId,
};

use crate::quit::{QuitOptions, QuitResult, QuitStatus};
use crate::reload::{ReloadError, ReloadRequest, ReloadStatus, WorkspaceCheckpoint};
use crate::tasks::{AppHandle, HostVisibility};

impl<R: Route> Router<R> {
    /// Identity of this live runtime, stable until host teardown or full reload.
    pub fn runtime_id(&self) -> RuntimeId {
        self.0.runtime_id()
    }

    /// Registers an access guard before the operation driver starts.
    pub fn entry_guard<F, Fut>(&self, guard: F) -> Result<(), OperationError>
    where
        F: Fn(GuardContext<R>) -> Fut + 'static,
        Fut: Future<Output = GuardResult> + 'static,
    {
        self.0.entry_guard(guard)
    }

    /// Registers typed, side-effect-free route admission before the operation driver starts.
    pub fn entry_admission<F, Fut>(&self, admission: F) -> Result<(), OperationError>
    where
        F: Fn(crate::router::AdmissionContext<R>) -> Fut + 'static,
        Fut: Future<Output = crate::router::Admission<R>> + 'static,
    {
        self.0.entry_admission(move |context| {
            admission(super::AdmissionContext {
                operation_id: context.operation_id,
                window_id: context.window_id,
                target_tab: context.target_tab,
                destination: context.destination,
                services: crate::services::ServiceRegistry::from_core(context.services),
            })
        })
    }

    /// Returns retained navigation statuses for one presenting window.
    pub fn navigation(&self, window: WindowId) -> Vec<NavigationStatus> {
        self.0.navigation(window)
    }

    /// Retries an exact retained navigation through the normal admission and guard chain.
    pub fn retry_navigation(
        &self,
        key: &NavigationKey,
    ) -> Result<Operation<Opened>, OperationError> {
        self.0.retry_navigation(key)
    }

    /// Removes one exact retained navigation status.
    pub fn dismiss_navigation(&self, key: &NavigationKey) -> Result<(), OperationError> {
        self.0.dismiss_navigation(key)
    }

    /// Registers an application-wide guard for every future quit.
    ///
    /// Guards run after finite work has settled or been cancelled and after all
    /// retained entries have approved their disposal. They run immediately
    /// before the router enters `Stopping`; return an error to keep the runtime
    /// open. The callback owns its application state and must re-read mutable
    /// state before approving, because the router cannot revalidate state that
    /// lives outside its model.
    ///
    /// Register guards before [`Router::run`](Self::run) starts driving the
    /// runtime. Registration after that point returns `InvalidOptions`.
    pub fn quit_guard<F, Fut>(&self, guard: F) -> Result<(), OperationError>
    where
        F: Fn(QuitGuardContext) -> Fut + 'static,
        Fut: Future<Output = Result<(), GuardReason>> + 'static,
    {
        self.0.quit_guard(guard)
    }

    /// Installs the browser/embedding history sink. Failures are exposed in snapshots.
    pub fn on_history(&self, sink: impl Fn(HistoryChange) -> Result<(), String> + 'static) {
        self.0.on_history(sink)
    }

    /// Copies state without exposing retained views or mutable histories.
    pub fn snapshot(&self) -> RouterSnapshot {
        self.0.snapshot()
    }

    /// Returns retained operation state or an explicit expiration/unknown error.
    pub fn operation(&self, id: OperationId) -> Result<OperationState, OperationError> {
        self.0.operation(id)
    }

    /// Finds a retained entry handle by identity, including non-current history entries.
    pub fn entry(&self, id: EntryId) -> Result<EntryHandle<R>, OperationError> {
        self.0.entry(id)
    }

    /// Waits for a newer owned snapshot revision without a snapshot/subscription gap.
    pub async fn changed(&self, revision: u64) -> u64 {
        self.0.changed(revision).await
    }

    /// Requests cancellation. Returns false when the operation already finished.
    pub fn cancel(&self, id: OperationId) -> Result<bool, OperationError> {
        self.0.cancel(id)
    }

    /// Stops admission and cancels pending operations. Host termination is not a guarded quit.
    pub fn shutdown(&self) {
        self.0.shutdown()
    }

    /// Drives eagerly submitted jobs. Conflicting tab jobs run in order; other tabs continue.
    /// The host owns this future and must keep polling it for the runtime's lifetime.
    pub async fn run(&self) -> Result<(), OperationError> {
        self.0.run().await
    }

    /// Application-owned tasks survive internal tab close; their origin is retained.
    pub fn app(&self) -> AppHandle {
        self.0.app()
    }

    /// Supplies an actual host visibility signal, independently of keyboard focus.
    /// Unknown assumes visible while exposing that visibility is unmeasured.
    pub fn set_host_visibility(&self, visibility: HostVisibility) {
        self.0.set_host_visibility(visibility)
    }

    /// Installs guarded host close and, when supported, additional window creation.
    /// Install before the driver starts. The route mount adapter must mount each
    /// destination in its navigator's window.
    ///
    /// # Errors
    /// Rejects repeat installation, a running driver, or multi-window support in a browser.
    pub fn window_host(&self, host: impl WindowHost) -> Result<(), OperationError> {
        self.0.window_host(host)
    }

    /// Advertised targets; single-window and browser hosts omit NewWindow.
    pub fn supported_targets(&self) -> Vec<OpenTarget> {
        self.0.supported_targets()
    }

    /// Copies one host's tabs, selection and decisions without changing global state.
    pub fn snapshot_in_window(&self, window: WindowId) -> Result<RouterSnapshot, OperationError> {
        self.0.snapshot_in_window(window)
    }

    /// Guards every retained entry before disposing this window atomically.
    /// Other windows and application-owned tasks survive. Repeated pending closes
    /// join the same operation. Cancellation/denial leaves the complete window intact.
    pub fn close_window(&self, window: WindowId) -> Operation<WindowClosed> {
        self.0.close_window(window)
    }

    /// Reports visibility for exactly one window; hiding it does not stop another host.
    pub fn set_window_visibility(
        &self,
        window: WindowId,
        visibility: HostVisibility,
    ) -> Result<(), OperationError> {
        self.0.set_window_visibility(window, visibility)
    }

    /// Latest quit phase and live finite-work identities. Ready remains queryable until host exit.
    pub fn quit_status(&self) -> QuitStatus {
        self.0.quit_status()
    }

    /// Attaches one staged update to a new guarded quit operation.
    ///
    /// Existing document, history, window and active-work guards run unchanged.
    /// Decline/cancellation drops the handoff without installation. Hosts commit
    /// it with `complete_update_restart` only after `QuitPhase::Ready`.
    /// # Errors
    /// Rejects concurrent quits or a quit that was not admitted.
    #[cfg(feature = "updates")]
    pub fn quit_for_update(
        &self,
        handoff: rake_updates::RestartHandoff,
        options: QuitOptions,
    ) -> Result<Operation<QuitResult>, rake_updates::Error> {
        self.0.quit_for_update(handoff, options)
    }

    /// Runs a pending updater after all guards and managed work have completed.
    /// Native Rake workspaces call this before host exit; custom hosts call it there too.
    /// Returns the native SDK/host termination disposition, retained across windows.
    /// # Errors
    /// Rejects premature calls and preserves native handoff failures for manual restart.
    #[cfg(feature = "updates")]
    pub fn complete_update_restart(
        &self,
    ) -> Result<rake_updates::RestartDisposition, rake_updates::Error> {
        self.0.complete_update_restart()
    }

    /// Returns the updater's failed restart diagnostic, for a manual-restart UI.
    #[cfg(feature = "updates")]
    pub fn update_restart_error(&self) -> Option<String> {
        self.0.update_restart_error()
    }

    /// Eagerly submits one guarded quit. Concurrent requests join its operation;
    /// the first request's job policy wins. Dropping handles never cancels quit.
    pub fn quit(&self, options: QuitOptions) -> Operation<QuitResult> {
        self.0.quit(options)
    }

    /// Current reload state. Ready means managed futures have been dropped and the
    /// host UI has acknowledged detachment; it does not prove native code unload safety.
    pub fn reload_status(&self) -> ReloadStatus {
        self.0.reload_status()
    }

    /// Stops admission and invalidates publications before cancelling old managed work.
    /// Busy operations/decisions must settle explicitly; preparation never cancels saves.
    pub fn prepare_reload(&self, request: ReloadRequest) -> Result<ReloadStatus, ReloadError> {
        self.0.prepare_reload(request)
    }

    /// Captures a prepared workspace on the UI thread. Application providers can fail;
    /// the supervisor must abort or preserve that failure, never silently restart empty.
    pub fn reload_checkpoint(
        &self,
        application_id: &str,
    ) -> Result<WorkspaceCheckpoint, ReloadError> {
        self.0.reload_checkpoint(application_id)
    }

    /// Resumes eligible factories after abort or supported in-place replacement.
    /// `applied=true` is an assertion by the external native reloader: it owns code
    /// replacement, ABI compatibility and retirement of every old callback/destructor.
    pub fn finish_reload(&self, id: &str, applied: bool) -> Result<ReloadStatus, ReloadError> {
        self.0.finish_reload(id, applied)
    }

    /// Reconstructs an empty replacement workspace. Validates every route/identity
    /// before mounting; mounts are provisional until all checkpoint consumers succeed.
    /// On failure, no partial workspace is attached and the caller retains its checkpoint.
    pub async fn restore_workspace(
        &self,
        checkpoint: &WorkspaceCheckpoint,
        application_id: &str,
    ) -> Result<(), ReloadError> {
        self.0.restore_workspace(checkpoint, application_id).await
    }

    /// Returns the application's shared update model, when installed policy is available.
    #[cfg(feature = "updates")]
    pub fn updates(&self) -> Option<Rc<rake_updates::UpdateController>> {
        self.0.updates()
    }

    /// Checks the installed provider without downloading or requesting termination.
    /// # Errors
    /// Rejects unavailable or externally managed providers and concurrent operations.
    #[cfg(feature = "updates")]
    pub fn check_for_updates(&self) -> Result<(), rake_updates::Error> {
        self.0.check_for_updates()
    }

    /// Downloads the selected update; Sparkle uses its native review UI.
    /// # Errors
    /// Rejects missing availability or another active operation.
    #[cfg(feature = "updates")]
    pub fn download_update(&self) -> Result<(), rake_updates::Error> {
        self.0.download_update()
    }

    /// Submits verified staging to the existing save, work and window quit guards.
    /// # Errors
    /// Rejects absent staging or a competing quit operation.
    #[cfg(feature = "updates")]
    pub fn restart_for_update(
        &self,
        options: QuitOptions,
    ) -> Result<Operation<QuitResult>, rake_updates::Error> {
        self.0.restart_for_update(options)
    }
}

impl<R: Route> Navigator<R> {
    /// Dispatches typed application arguments through the same operation as the CLI.
    /// The page keeps only its weak navigator, avoiding a retained-view ownership cycle.
    pub fn dispatch<C: crate::control::ApplicationCommand>(
        &self,
        command: C,
        key: String,
    ) -> Result<crate::control::Reply, crate::control::ControlError> {
        self.0.dispatch(command, key)
    }

    /// Application work with this navigator's original causal tab and entry IDs.
    pub fn app(&self) -> AppHandle {
        self.0.app()
    }

    /// Eagerly opens a destination with default activation and tab mode.
    pub fn open(&self, destination: R, target: OpenTarget) -> Operation<Opened> {
        self.0.open(destination, target)
    }

    /// Eagerly submits explicit creation options; new tabs never silently deduplicate.
    pub fn open_with(&self, destination: R, options: OpenOptions) -> Operation<Opened> {
        self.0.open_with(destination, options)
    }

    /// Resolves this navigator's current committed tab without cloning workspace history.
    /// Page-bound navigators retain their originating tab; window-bound navigators
    /// resolve that window's selection when called.
    /// # Errors
    /// Returns `NoCurrentTab`, a closed-target error, or a lifecycle admission error.
    pub fn current_tab(&self) -> Result<TabHandle<R>, OperationError> {
        self.0.current_tab()
    }

    /// Targets an exact tab without semantic reuse.
    pub fn tab(&self, tab_id: TabId) -> TabHandle<R> {
        self.0.tab(tab_id)
    }

    /// Reuses the most recently selected matching current entry, or creates one tab.
    pub fn reveal(&self, destination: R, scope: RevealScope) -> Operation<Revealed> {
        self.0.reveal(destination, scope)
    }

    /// Restores a browser checkpoint without pushing another browser visit.
    pub fn restore(&self, checkpoint: BrowserCheckpoint) -> Operation<Revealed> {
        self.0.restore(checkpoint)
    }

    /// Restores a decoded deep link without adding a browser visit or destroying other tabs.
    pub fn restore_destination(&self, destination: R) -> Operation<Revealed> {
        self.0.restore_destination(destination)
    }

    /// Resolves one currently valid decision. Unknown choices leave the decision pending.
    pub fn respond(&self, id: DecisionId, choice: &str) -> Result<(), DecisionError> {
        self.0.respond(id, choice)
    }

    /// Closes this navigator's concrete window, independently of keyboard focus.
    pub fn close_window(&self) -> Operation<WindowClosed> {
        self.0.close_window()
    }

    /// Host captured by this navigator; it is never inferred from current keyboard focus.
    pub fn window_id(&self) -> WindowId {
        self.0.window_id()
    }

    /// Requests the same application-wide guarded quit as the CLI and native close handler.
    pub fn quit(&self, options: QuitOptions) -> Operation<QuitResult> {
        self.0.quit(options)
    }
}
