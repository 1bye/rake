//! Physical host capabilities and atomic, revision-checked disposal of one window.

use super::{
    Action, Activation, CancelReason, CancellationToken, Core, DisposeReason, Either, FutureExt,
    GuardKind, HostVisibility, Job, Lane, Navigator, OpenTarget, Operation, OperationError,
    OperationId, OperationState, Outcome, Rc, Route, Router, RouterSnapshot, Stop, WindowClosed,
    WindowHost, WindowId, WindowState, guard, oneshot, select,
};

#[doc(hidden)]
pub trait HostBinding: std::any::Any {
    fn detach_window(&self, id: WindowId);

    fn shutdown(&self);

    fn begin_reload(&self, windows: Vec<WindowId>);

    fn reload_ready(&self, visibility: &dyn Fn(WindowId) -> Option<HostVisibility>) -> bool;
}

impl<R: Route> Router<R> {
    #[doc(hidden)]
    pub fn bind_host(&self, host: impl HostBinding) {
        self.0.host_binding.get_or_init(|| Box::new(host));
    }

    #[doc(hidden)]
    pub fn host_binding(&self) -> Option<&dyn HostBinding> {
        self.0.host_binding.get().map(Box::as_ref)
    }

    #[doc(hidden)]
    pub fn set_initial_window(&self, id: WindowId) {
        self.0.initial_window.set(id);
        self.0.state.borrow_mut().windows =
            std::collections::BTreeMap::from([(id, WindowState::default())]);
    }

    #[doc(hidden)]
    pub fn set_host_attached(&self, attached: bool) {
        self.0.has_window.set(attached);
    }

    #[cfg(not(target_family = "wasm"))]
    #[doc(hidden)]
    pub fn request_host_close(&self, id: WindowId) -> bool {
        if self.0.quit.borrow().phase == crate::quit::QuitPhase::Ready {
            return true;
        }

        if self.0.window_host.borrow().is_none() {
            drop(self.quit(crate::quit::QuitOptions::default()));

            return false;
        }

        let opening = {
            let state = self.0.state.borrow();

            let Some(window) = state.windows.get(&id) else {
                return true;
            };

            window.opening
        };

        if let Some(operation) = opening {
            let _ = self.cancel(operation);
        } else {
            drop(self.close_window(id));
        }

        false
    }

    #[cfg(target_family = "wasm")]
    #[doc(hidden)]
    pub fn warn_before_unload(&self) -> bool {
        if self.0.page_reload.replace(false) {
            return false;
        }

        let snapshot = self.snapshot();

        snapshot.quit.phase != crate::quit::QuitPhase::Ready
            && (snapshot.quit.phase.is_pending()
                || snapshot
                    .tabs
                    .iter()
                    .flat_map(|tab| &tab.entries)
                    .any(|entry| entry.dirty)
                || !snapshot.quit.pending_tasks.is_empty()
                || !snapshot.quit.pending_operations.is_empty()
                || !self.0.quit_guards.borrow().is_empty())
    }

    /// Installs guarded host close and, when supported, additional window creation.
    /// Install before the driver starts. The route mount adapter must mount each
    /// destination in its navigator's window.
    ///
    /// # Errors
    /// Rejects repeat installation, a running driver, or multi-window support in a browser.
    pub fn window_host(&self, host: impl WindowHost) -> Result<(), OperationError> {
        if (cfg!(target_family = "wasm") && host.supports_new_windows())
            || self.0.driving.get()
            || self.0.window_host.borrow().is_some()
        {
            return Err(OperationError::InvalidOptions);
        }

        *self.0.window_host.borrow_mut() = Some(Rc::new(host));

        Ok(())
    }

    /// Advertised targets; single-window and browser hosts omit NewWindow.
    pub fn supported_targets(&self) -> Vec<OpenTarget> {
        let mut targets = vec![OpenTarget::CurrentTab, OpenTarget::NewTab];

        if self.0.supports_new_windows() {
            targets.push(OpenTarget::NewWindow);
        }

        targets
    }

    #[doc(hidden)]
    pub fn presented_snapshot(&self, window: WindowId) -> RouterSnapshot {
        self.snapshot_in_window(window).unwrap_or_else(|_| {
            let mut snapshot = self.snapshot();

            snapshot.window_id = window;
            snapshot.tabs.clear();
            snapshot.decisions.clear();
            snapshot.selected = None;

            snapshot
        })
    }

    /// Copies one host's tabs, selection and decisions without changing global state.
    pub fn snapshot_in_window(&self, window: WindowId) -> Result<RouterSnapshot, OperationError> {
        self.check_window(window)?;

        Ok(self.snapshot_filtered(Some(window)))
    }

    #[doc(hidden)]
    pub fn check_window(&self, window: WindowId) -> Result<(), OperationError> {
        if self.0.state.borrow().windows.contains_key(&window) {
            Ok(())
        } else {
            Err(OperationError::WindowUnavailable)
        }
    }

    /// Guards every retained entry before disposing this window atomically.
    /// Other windows and application-owned tasks survive. Repeated pending closes
    /// join the same operation. Cancellation/denial leaves the complete window intact.
    pub fn close_window(&self, window: WindowId) -> Operation<WindowClosed> {
        if let Some(handle) = self.0.window_closes.borrow().get(&window) {
            return handle.clone();
        }

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

        let admission = self.navigator().for_window(window).core().and_then(|_| {
            if self.0.window_host.borrow().is_none() {
                return Err(OperationError::UnsupportedCapability(OpenTarget::NewWindow));
            }

            if self.0.state.borrow().tokens.len() >= self.0.config.pending_operations {
                return Err(OperationError::QueueFull);
            }

            Ok(())
        });

        if let Err(error) = admission {
            let result = Err(error);

            self.0.finish::<WindowClosed>(id, &result);
            let _ = sender.send(result);

            return handle;
        }

        let lanes = {
            let mut state = self.0.state.borrow_mut();

            state
                .windows
                .get_mut(&window)
                .expect("admitted window")
                .closing = Some(id);
            let mut lanes = vec![Lane::Window(window)];

            lanes.extend(
                state
                    .tabs
                    .iter()
                    .filter(|tab| tab.window == window)
                    .map(|tab| Lane::Tab(tab.id)),
            );

            lanes
        };

        self.0
            .window_closes
            .borrow_mut()
            .insert(window, handle.clone());
        let token = CancellationToken::new();

        self.0.state.borrow_mut().tokens.insert(id, token.clone());
        self.0.status(id, OperationState::Queued);
        self.0.event(id, "window_close_submitted");
        let weak = Rc::downgrade(&self.0);
        let cancellation = token.clone();

        let future = async move {
            let Some(core) = weak.upgrade() else {
                return;
            };

            core.status(id, OperationState::Running);
            let result = match select(
                Box::pin(cancellation.cancelled()),
                Box::pin(close(&core, id, window)),
            )
            .await
            {
                Either::Left(_) if core.shutdown.get() => Err(OperationError::RuntimeShutdown),
                Either::Left(_) => Ok(Outcome::Cancelled(CancelReason::Requested)),
                Either::Right((Ok(()), _)) => {
                    Ok(Outcome::Committed(WindowClosed { window_id: window }))
                }
                Either::Right((Err(Stop::Error(error)), _)) => Err(error),
                Either::Right((Err(Stop::Blocked(reason)), _)) => Ok(Outcome::Blocked(reason)),
                Either::Right((Err(Stop::Cancelled(reason)), _)) => Ok(Outcome::Cancelled(reason)),
            };

            if let Some(host) = core.state.borrow_mut().windows.get_mut(&window) {
                host.closing = None;
            }

            core.window_closes.borrow_mut().remove(&window);
            core.finish(id, &result);
            let _ = sender.send(result);
        }
        .boxed_local();

        self.0.state.borrow_mut().queue.push_back(Job {
            lanes,
            token,
            future,
        });
        self.0.work.notify(usize::MAX);

        handle
    }

    /// Reports visibility for exactly one window; hiding it does not stop another host.
    pub fn set_window_visibility(
        &self,
        window: WindowId,
        visibility: HostVisibility,
    ) -> Result<(), OperationError> {
        let mut state = self.0.state.borrow_mut();

        let host = state
            .windows
            .get_mut(&window)
            .ok_or(OperationError::WindowUnavailable)?;

        if host.visibility == visibility {
            return Ok(());
        }

        host.visibility = visibility;
        drop(state);
        self.0.tasks.window_visibility(window, visibility);
        self.0.sync_presentation(window);
        self.0.notify();

        Ok(())
    }
}

impl<R: Route> Navigator<R> {
    /// Temporarily upgrades this weak handle for custom native GPUI window hosts.
    /// The returned router keeps state alive; do not store it in the installed
    /// [`WindowHost`], which would retain the runtime through its own adapter.
    #[cfg(not(target_family = "wasm"))]
    pub fn router(&self) -> Option<Router<R>> {
        self.upgrade_router()
    }

    #[doc(hidden)]
    pub fn upgrade_router(&self) -> Option<Router<R>> {
        self.core.upgrade().map(Router)
    }

    /// Closes this navigator's concrete window, independently of keyboard focus.
    pub fn close_window(&self) -> Operation<WindowClosed> {
        if let Some(core) = self.core.upgrade() {
            return Router(core).close_window(self.window);
        }

        Operation {
            id: OperationId(0),
            result: futures::future::ready(Err(OperationError::RuntimeShutdown))
                .boxed_local()
                .shared(),
        }
    }

    /// Host captured by this navigator; it is never inferred from current keyboard focus.
    pub fn window_id(&self) -> WindowId {
        self.window
    }
}

async fn close<R: Route>(
    core: &Rc<Core<R>>,
    id: OperationId,
    window: WindowId,
) -> Result<(), Stop> {
    loop {
        let entries: Vec<_> = core
            .state
            .borrow()
            .tabs
            .iter()
            .filter(|tab| tab.window == window)
            .flat_map(|tab| tab.entries.clone())
            .collect();

        let mut approvals = Vec::new();

        for entry in &entries {
            approvals.push((
                entry.clone(),
                guard(
                    core,
                    id,
                    &Action::CloseWindow,
                    entry,
                    GuardKind::Dispose(DisposeReason::CloseWindow),
                    window,
                )
                .await?,
            ));
        }

        if core
            .state
            .borrow()
            .tokens
            .get(&id)
            .is_some_and(CancellationToken::is_cancelled)
        {
            return Err(Stop::Cancelled(CancelReason::Requested));
        }

        if approvals
            .iter()
            .any(|(entry, revision)| !entry.alive.get() || entry.revision.get() != *revision)
        {
            continue;
        }

        let mut state = core.state.borrow_mut();

        state
            .windows
            .remove(&window)
            .ok_or(OperationError::WindowUnavailable)?;
        let mut retained = Vec::new();
        let mut removed = Vec::new();

        for tab in std::mem::take(&mut state.tabs) {
            if tab.window == window {
                removed.push(tab);
            } else {
                retained.push(tab);
            }
        }

        state.tabs = retained;

        for entry in removed.iter().flat_map(|tab| &tab.entries) {
            state.invalidate(Some(entry.id));
        }

        drop(state);

        for tab in removed {
            tab.scope.close();

            for entry in tab.entries {
                entry.dispose();
            }
        }

        core.tasks.forget_window(window);
        core.remove_window_host(window);
        core.notify();

        return Ok(());
    }
}

impl<R: Route> Core<R> {
    pub(super) fn supports_new_windows(&self) -> bool {
        let host = self.window_host.borrow().clone();

        host.is_some_and(|host| host.supports_new_windows())
    }

    pub(super) fn remove_window_host(&self, id: WindowId) {
        let host = self.window_host.borrow().clone();

        if let Some(host) = host {
            host.close(id);
        }

        if let Some(binding) = self.host_binding.get() {
            binding.detach_window(id);
        }
    }
}

pub(super) struct ProvisionalWindow<R: Route> {
    core: Rc<Core<R>>,
    id: WindowId,
    opened: bool,
    pub(super) committed: bool,
}

impl<R: Route> ProvisionalWindow<R> {
    pub(super) fn new(core: Rc<Core<R>>, id: WindowId, operation: OperationId) -> Self {
        core.state.borrow_mut().windows.insert(
            id,
            WindowState {
                opening: Some(operation),
                ..WindowState::default()
            },
        );

        Self {
            core,
            id,
            opened: false,
            committed: false,
        }
    }

    pub(super) async fn open(&mut self, activation: Activation) -> Result<(), Stop> {
        if !self.opened {
            let host = self
                .core
                .window_host
                .borrow()
                .clone()
                .ok_or(OperationError::UnsupportedCapability(OpenTarget::NewWindow))?;

            host.open(self.id, activation)
                .await
                .map_err(OperationError::Mount)?;
            self.opened = true;
        }

        Ok(())
    }
}

impl<R: Route> Drop for ProvisionalWindow<R> {
    fn drop(&mut self) {
        if !self.committed {
            self.core.state.borrow_mut().windows.remove(&self.id);
            self.core.tasks.forget_window(self.id);
            self.core.remove_window_host(self.id);
            self.core.notify();
        }
    }
}
