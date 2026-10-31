//! One global quit operation reuses entry decisions and coordinates existing work.

use super::{
    Action, Core, DisposeReason, GuardKind, Job, Navigator, Operation, OperationError, OperationId,
    OperationState, Outcome, Route, Router, Stop, guard,
};

use crate::http::CancellationToken;
use crate::quit::{QuitJobs, QuitOptions, QuitPhase, QuitResult, QuitStatus};
use crate::tasks::{TaskKind, TaskState};

use futures::{
    FutureExt,
    channel::oneshot,
    future::{Either, select},
};

use std::rc::Rc;

impl<R: Route> Core<R> {
    pub(super) fn quit_status(&self, work: &crate::tasks::WorkSnapshot) -> QuitStatus {
        let mut status = self.quit.borrow().clone();

        status.pending_operations = self
            .state
            .borrow()
            .tokens
            .keys()
            .copied()
            .filter(|id| Some(*id) != status.operation_id)
            .collect();
        status.pending_tasks = work
            .tasks
            .iter()
            .filter(|task| {
                task.kind == TaskKind::Task
                    && matches!(
                        task.state,
                        TaskState::Queued | TaskState::Running | TaskState::Cancelling
                    )
            })
            .map(|task| task.id)
            .collect();

        status
    }
}

impl<R: Route> Router<R> {
    /// Latest quit phase and live finite-work identities. Ready remains queryable until host exit.
    pub fn quit_status(&self) -> QuitStatus {
        self.0.quit_status(&self.0.tasks.snapshot())
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
        if self.quit_status().phase.blocks_navigation() {
            return Err(rake_updates::Error::Policy(
                "another quit already owns the application lifecycle".into(),
            ));
        }
        let operation = self.quit(options);
        if self.quit_status().operation_id != Some(operation.id())
            || !self.quit_status().phase.is_pending()
        {
            return Err(rake_updates::Error::Policy(
                "update quit was not admitted".into(),
            ));
        }
        *self.0.update_handoff.borrow_mut() = Some(handoff);
        *self.0.update_restart_error.borrow_mut() = None;
        self.0
            .update_exit
            .set(rake_updates::RestartDisposition::ExitHost);
        Ok(operation)
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
        if self.quit_status().phase != QuitPhase::Ready {
            return Err(rake_updates::Error::Policy(
                "update restart requires a committed guarded quit".into(),
            ));
        }
        if let Some(error) = self.update_restart_error() {
            return Err(rake_updates::Error::Provider(error));
        }
        let Some(handoff) = self.0.update_handoff.borrow_mut().take() else {
            return Ok(self.0.update_exit.get());
        };
        match handoff.commit(true) {
            Ok(disposition) => {
                self.0.update_exit.set(disposition);
                Ok(disposition)
            }
            Err(error) => {
                *self.0.update_restart_error.borrow_mut() = Some(error.to_string());
                self.0.notify();
                Err(error)
            }
        }
    }

    /// Returns the updater's failed restart diagnostic, for a manual-restart UI.
    #[cfg(feature = "updates")]
    pub fn update_restart_error(&self) -> Option<String> {
        self.0.update_restart_error.borrow().clone()
    }

    /// Eagerly submits one guarded quit. Concurrent requests join its operation;
    /// the first request's job policy wins. Dropping handles never cancels quit.
    pub fn quit(&self, options: QuitOptions) -> Operation<QuitResult> {
        if self.0.quit.borrow().phase.blocks_navigation()
            && let Some(handle) = self.0.quit_handle.borrow().as_ref()
        {
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

        let admission = if self.is_shutdown() {
            Err(OperationError::RuntimeShutdown)
        } else if self.0.reload.borrow().request.is_some()
            || self.0.reload.borrow().phase == crate::reload::ReloadPhase::Restoring
        {
            Err(OperationError::Reloading)
        } else if self.0.state.borrow().tokens.len() >= self.0.config.pending_operations {
            Err(OperationError::QueueFull)
        } else {
            Ok(())
        };

        if let Err(error) = admission {
            let result = Err(error);

            self.0.finish::<QuitResult>(id, &result);
            let _ = sender.send(result);

            return handle;
        }

        *self.0.quit.borrow_mut() = QuitStatus {
            phase: QuitPhase::WaitingForJobs,
            operation_id: Some(id),
            options,
            ..QuitStatus::default()
        };
        *self.0.quit_handle.borrow_mut() = Some(handle.clone());
        let token = CancellationToken::new();

        self.0.state.borrow_mut().tokens.insert(id, token.clone());
        self.0.status(id, OperationState::Queued);
        self.0.event(id, "quit_submitted");
        let weak = Rc::downgrade(&self.0);
        let cancellation = token.clone();

        let future = async move {
            let Some(core) = weak.upgrade() else {
                return;
            };

            core.status(id, OperationState::Running);
            let execution = execute(&core, id, options);

            let result = match select(Box::pin(cancellation.cancelled()), Box::pin(execution)).await
            {
                Either::Left(_) if core.shutdown.get() => Err(OperationError::RuntimeShutdown),
                Either::Left(_) => Ok(Outcome::Cancelled(super::CancelReason::Requested)),
                Either::Right((result, _)) => result,
            };

            // A guard can request cancellation synchronously during its own poll.
            let result = if cancellation.is_cancelled() {
                if core.shutdown.get() {
                    Err(OperationError::RuntimeShutdown)
                } else {
                    Ok(Outcome::Cancelled(super::CancelReason::Requested))
                }
            } else {
                result
            };

            let phase = match &result {
                Ok(Outcome::Committed(_)) => QuitPhase::Ready,
                Ok(Outcome::Cancelled(_)) => QuitPhase::Cancelled,
                _ => QuitPhase::Blocked,
            };

            #[cfg(feature = "updates")]
            if phase != QuitPhase::Ready {
                core.update_handoff.borrow_mut().take();
            }
            core.quit.borrow_mut().phase = phase;
            core.finish(id, &result);
            let _ = sender.send(result);
        }
        .boxed_local();

        // Quit cannot hold tab lanes while saves/older operations finish.
        self.0.state.borrow_mut().queue.push_back(Job {
            lanes: Vec::new(),
            token,
            future,
        });
        self.0.work.notify(usize::MAX);

        handle
    }
}

impl<R: Route> Navigator<R> {
    /// Requests the same application-wide guarded quit as the CLI and native close handler.
    pub fn quit(&self, options: QuitOptions) -> Operation<QuitResult> {
        // An expired weak navigator must not create another runtime or retarget a quit.
        if let Some(core) = self.core.upgrade() {
            return Router(core).quit(options);
        }

        Operation {
            id: OperationId(0),
            result: futures::future::ready(Err(OperationError::RuntimeShutdown))
                .boxed_local()
                .shared(),
        }
    }
}

async fn wait_jobs<R: Route>(router: &Router<R>, policy: QuitJobs, tasks: bool) {
    loop {
        let work = router.0.tasks.snapshot();
        let revision = router.0.state.borrow().revision + work.revision;
        let status = router.0.quit_status(&work);

        if status.pending_operations.is_empty()
            && (!tasks || status.pending_tasks.is_empty())
            && work.pending_publications == 0
        {
            return;
        }

        if policy == QuitJobs::Cancel {
            for operation in status.pending_operations {
                let _ = router.cancel(operation);
            }

            if tasks {
                for task in status.pending_tasks {
                    let _ = router.cancel_task(task);
                }
            }
        }

        // Decisions, job completion and publications all wake the same state watch.
        router.changed(revision).await;
    }
}

async fn execute<R: Route>(
    core: &Rc<Core<R>>,
    id: OperationId,
    options: QuitOptions,
) -> Result<Outcome<QuitResult>, OperationError> {
    let result = transition(core, id, options).await;

    match result {
        Ok(()) => Ok(Outcome::Committed(QuitResult {
            runtime_id: core.runtime,
        })),
        Err(Stop::Cancelled(reason)) => Ok(Outcome::Cancelled(reason)),
        Err(Stop::Blocked(reason)) => Ok(Outcome::Blocked(reason)),
        Err(Stop::Error(error)) => Err(error),
    }
}

async fn transition<R: Route>(
    core: &Rc<Core<R>>,
    id: OperationId,
    options: QuitOptions,
) -> Result<(), Stop> {
    let router = Router(core.clone());

    loop {
        core.quit.borrow_mut().phase = QuitPhase::WaitingForJobs;
        core.notify();
        wait_jobs(&router, options.jobs, false).await;
        core.quit.borrow_mut().phase = QuitPhase::Guarding;
        core.notify();
        let entries: Vec<_> = core
            .state
            .borrow()
            .tabs
            .iter()
            .flat_map(|tab| tab.entries.clone())
            .collect();

        let mut approvals = Vec::new();

        for entry in &entries {
            let revision = guard(
                core,
                id,
                &Action::Quit,
                entry,
                GuardKind::Dispose(DisposeReason::QuitApplication),
                entry.window,
            )
            .await?;

            approvals.push((entry.clone(), revision));
        }

        core.quit.borrow_mut().phase = QuitPhase::WaitingForJobs;
        core.notify();
        wait_jobs(&router, options.jobs, true).await;

        if approvals
            .iter()
            .any(|(entry, revision)| !entry.alive.get() || entry.revision.get() != *revision)
        {
            continue;
        }

        // Last cancellation check, including a request made synchronously by a guard.
        if core
            .state
            .borrow()
            .tokens
            .get(&id)
            .is_some_and(|token| token.is_cancelled())
        {
            return Err(Stop::Cancelled(super::CancelReason::Requested));
        }

        core.state.borrow_mut().tokens.remove(&id);
        core.quit.borrow_mut().phase = QuitPhase::Stopping;
        core.notify();
        core.tasks.shutdown();

        loop {
            let revision = core.tasks.revision();

            if core.tasks.quiescent() {
                break;
            }

            core.tasks.changed(revision).await;
        }

        let tabs = std::mem::take(&mut core.state.borrow_mut().tabs);

        for window in core.state.borrow_mut().windows.values_mut() {
            window.selected = None;
        }

        for tab in &tabs {
            tab.scope.close();

            for entry in &tab.entries {
                entry.dispose();
            }
        }

        drop(tabs);
        core.emit_history(true);

        return Ok(());
    }
}

#[cfg(feature = "gpui")]
impl<R: Route> Router<R> {
    pub(crate) fn install_quit_action(&self, cx: &mut gpui::App) {
        #[cfg(all(feature = "native-lifecycle", target_os = "macos"))]
        if self.0.native_quit.borrow().is_none() {
            match rake_updates::NativeQuitBridge::new() {
                Ok(bridge) => *self.0.native_quit.borrow_mut() = Some(Rc::new(bridge)),
                Err(error) => tracing::error!(%error,"native quit guard could not be installed"),
            }
        }
        let core = Rc::downgrade(&self.0);

        cx.on_action(move |_: &crate::quit::RequestQuit, _| {
            if let Some(core) = core.upgrade() {
                drop(Router(core).quit(QuitOptions::default()));
            }
        });

        #[cfg(not(target_family = "wasm"))]
        {
            cx.bind_keys([gpui::KeyBinding::new(
                if cfg!(target_os = "macos") {
                    "cmd-q"
                } else {
                    "ctrl-q"
                },
                crate::quit::RequestQuit,
                None,
            )]);
            cx.set_menus([gpui::Menu {
                name: "Application".into(),
                items: vec![gpui::MenuItem::action("Quit", crate::quit::RequestQuit)],
                disabled: false,
            }]);
        }
    }
}

#[cfg(all(feature = "native-lifecycle", feature = "gpui", target_os = "macos"))]
impl<R: Route> Router<R> {
    pub(crate) fn native_quit_changed(&self) -> futures::future::LocalBoxFuture<'static, ()> {
        let changed = self
            .0
            .native_quit
            .borrow()
            .as_ref()
            .map(|bridge| bridge.changed());
        async move {
            if let Some(changed) = changed {
                changed.await;
            } else {
                std::future::pending::<()>().await;
            }
        }
        .boxed_local()
    }

    #[cfg(not(feature = "updates"))]
    pub(crate) async fn changed_with_native_quit(&self, revision: u64) -> u64 {
        let changed = self.changed(revision);
        let _ = select(Box::pin(changed), self.native_quit_changed()).await;
        self.poll_native_quit();
        self.revision()
    }

    pub(crate) fn poll_native_quit(&self) {
        let requested = self
            .0
            .native_quit
            .borrow()
            .as_ref()
            .is_some_and(|bridge| bridge.take_request());
        if requested {
            drop(self.quit(QuitOptions::default()));
        }
        if matches!(
            self.quit_status().phase,
            QuitPhase::Cancelled | QuitPhase::Blocked
        ) {
            self.reply_native_quit(false);
        }
    }
    pub(crate) fn prepare_native_exit(&self, approved: bool) {
        if let Some(bridge) = self.0.native_quit.borrow().as_ref() {
            bridge.prepare_exit(approved);
        }
    }
    pub(crate) fn reply_native_quit(&self, approved: bool) {
        if let Some(bridge) = self.0.native_quit.borrow().as_ref() {
            bridge.reply(approved);
        }
    }
}
