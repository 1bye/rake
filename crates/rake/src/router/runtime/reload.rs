//! Owned development state and the quiescent reload boundary.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;

use crate::http::CancellationToken;
use futures::future::{Either, select};

use crate::reload::{
    Checkpoint, EntryCheckpoint, MAX_CHECKPOINT_BYTES, ReloadError, ReloadPhase, ReloadRequest,
    ReloadState, ReloadStatus, TabCheckpoint, WorkspaceCheckpoint,
};

use crate::router::{GenerationId, Route, TabMode};
use crate::tasks::{HostVisibility, TaskOwner};

use super::{Entry, EntryHandle, Router, Tab};

impl<R: Route> EntryHandle<R> {
    /// Installs the entry's synchronous owned-state provider. Capture logical data,
    /// not a strong entity/runtime handle. Dirty entries require a provider to reload.
    pub fn on_reload(
        &self,
        capture: impl Fn() -> Result<Checkpoint, ReloadError> + 'static,
    ) -> Result<(), ReloadError> {
        let entry = self
            .live()
            .map_err(|error| ReloadError::State(error.to_string()))?;

        *entry.reload_capture.borrow_mut() = Some(Rc::new(capture));

        Ok(())
    }

    /// Decodes replacement state during mount. An unconsumed checkpoint rejects
    /// restoration, preventing a replacement from silently discarding supported state.
    pub fn reload_state<T: ReloadState>(&self) -> Result<Option<T>, ReloadError> {
        let entry = self
            .live()
            .map_err(|error| ReloadError::State(error.to_string()))?;

        let state = entry
            .restored_state
            .borrow()
            .clone()
            .map(T::restore)
            .transpose()?;

        entry.state_consumed.set(true);

        Ok(state)
    }
}

impl<R: Route> Router<R> {
    // Only the bridge calls this, after saving a checkpoint (or retaining a failed restore).
    #[cfg(target_family = "wasm")]
    pub(crate) fn reload_page(&self) -> Result<(), String> {
        self.0.page_reload.set(true);
        let result = web_sys::window()
            .ok_or_else(|| "browser unavailable".to_string())
            .and_then(|window| {
                window
                    .location()
                    .reload()
                    .map_err(|error| format!("{error:?}"))
            });

        if result.is_err() {
            self.0.page_reload.set(false);
        }

        result
    }

    /// Current reload state. Ready means managed futures have been dropped and the
    /// host UI has acknowledged detachment; it does not prove native code unload safety.
    pub fn reload_status(&self) -> ReloadStatus {
        let mut status = self.0.reload.borrow().clone();

        let ready = {
            let state = self.0.state.borrow();

            self.0.reload_ui_ready.borrow().iter().all(|id| {
                state
                    .windows
                    .get(id)
                    .is_none_or(|window| window.visibility == HostVisibility::Hidden)
            })
        };

        if status.phase == ReloadPhase::Quiescing && self.0.tasks.quiescent() && ready {
            status.phase = ReloadPhase::Ready;
        }

        status
    }

    /// Stops admission and invalidates publications before cancelling old managed work.
    /// Busy operations/decisions must settle explicitly; preparation never cancels saves.
    pub fn prepare_reload(&self, request: ReloadRequest) -> Result<ReloadStatus, ReloadError> {
        if request.id.is_empty() || request.id.len() > 128 || self.is_shutdown() {
            return Err(ReloadError::Incompatible(
                "invalid reload request or stopped host".into(),
            ));
        }

        if self.0.reload.borrow().request.as_ref() == Some(&request) {
            return Ok(self.reload_status());
        }

        let state = self.0.state.borrow();

        if self.0.quit.borrow().phase.blocks_navigation()
            || !state.tokens.is_empty()
            || !state.decisions.is_empty()
            || self.0.reload.borrow().request.is_some()
            || self.0.reload.borrow().phase == ReloadPhase::Restoring
        {
            return Err(ReloadError::Busy);
        }

        drop(state);
        let restored_from = self.0.reload.borrow().restored_from;

        *self.0.reload.borrow_mut() = ReloadStatus {
            restored_from,
            generation: self.0.tasks.generation(),
            phase: ReloadPhase::Quiescing,
            request: Some(request),
            error: None,
        };
        *self.0.reload_ui_ready.borrow_mut() = if self.0.has_window.get() {
            self.0.state.borrow().windows.keys().copied().collect()
        } else {
            BTreeSet::new()
        };
        self.0.tasks.pause();
        self.0.notify();

        Ok(self.reload_status())
    }

    #[cfg(feature = "gpui")]
    pub(crate) fn acknowledge_reload_ui(&self, window: super::WindowId) {
        if self.0.reload_ui_ready.borrow_mut().remove(&window) {
            self.0.notify();
        }
    }

    /// Captures a prepared workspace on the UI thread. Application providers can fail;
    /// the supervisor must abort or preserve that failure, never silently restart empty.
    pub fn reload_checkpoint(
        &self,
        application_id: &str,
    ) -> Result<WorkspaceCheckpoint, ReloadError> {
        if self.reload_status().phase != ReloadPhase::Ready {
            return Err(ReloadError::NotReady);
        }

        let revision = self.0.state.borrow().revision;

        let tabs: Vec<_> = self
            .0
            .state
            .borrow()
            .tabs
            .iter()
            .map(|tab| {
                (
                    tab.id,
                    tab.window,
                    tab.mode,
                    tab.cursor,
                    tab.selected_at,
                    tab.entries.clone(),
                )
            })
            .collect();

        let mut checkpoint_tabs = Vec::new();

        for (id, window_id, mode, cursor, selected_at, entries) in tabs {
            let mut captured = Vec::new();

            for entry in entries {
                let provider = entry.reload_capture.borrow().clone();
                let state = provider.map(|capture| capture()).transpose()?;

                if entry.dirty.get() && state.is_none() {
                    return Err(ReloadError::MissingState(entry.id));
                }

                captured.push(EntryCheckpoint {
                    id: entry.id,
                    destination: entry.destination.encode(),
                    dirty: entry.dirty.get(),
                    revision: entry.revision.get(),
                    state,
                });
            }

            checkpoint_tabs.push(TabCheckpoint {
                window_id,
                id,
                mode,
                cursor,
                selected_at,
                entries: captured,
            });
        }

        let state = self.0.state.borrow();

        if state.revision != revision {
            return Err(ReloadError::Busy);
        }

        let checkpoint = WorkspaceCheckpoint {
            version: 2,
            windows: state
                .windows
                .iter()
                .map(|(id, window)| crate::reload::WindowCheckpoint {
                    id: *id,
                    selected: window.selected,
                })
                .collect(),
            application_id: application_id.into(),
            source_runtime: self.runtime_id(),
            generation: self.0.tasks.generation(),
            run_sequence: self.0.tasks.run_sequence(),
            selected: state
                .windows
                .get(&self.0.initial_window.get())
                .and_then(|window| window.selected),
            tabs: checkpoint_tabs,
        };

        check_size(&checkpoint)?;

        Ok(checkpoint)
    }

    /// Resumes eligible factories after abort or supported in-place replacement.
    /// `applied=true` is an assertion by the external native reloader: it owns code
    /// replacement, ABI compatibility and retirement of every old callback/destructor.
    pub fn finish_reload(&self, id: &str, applied: bool) -> Result<ReloadStatus, ReloadError> {
        let status = self.reload_status();

        if status.request.as_ref().map(|request| request.id.as_str()) != Some(id) {
            return Err(ReloadError::WrongRequest);
        }

        if applied && status.phase != ReloadPhase::Ready {
            return Err(ReloadError::NotReady);
        }

        if applied
            && status.request.as_ref().is_some_and(|request| {
                request.mechanism != crate::reload::ReloadMechanism::NativeCodeReplacement
            })
        {
            return Err(ReloadError::Incompatible(
                "only an external native code replacement can apply in place".into(),
            ));
        }

        let generation = if applied {
            GenerationId(
                status
                    .generation
                    .0
                    .checked_add(1)
                    .ok_or_else(|| ReloadError::Incompatible("generation exhausted".into()))?,
            )
        } else {
            status.generation
        };

        *self.0.reload.borrow_mut() = ReloadStatus {
            restored_from: status.restored_from,
            generation,
            phase: if applied {
                ReloadPhase::Restored
            } else {
                ReloadPhase::Aborted
            },
            request: None,
            error: None,
        };
        #[cfg(target_family = "wasm")]
        self.0.page_reload.set(false);
        self.0.tasks.resume(generation, 0);
        self.0.notify();

        Ok(self.reload_status())
    }

    #[cfg(feature = "gpui")]
    pub(crate) fn reload_failed(&self, error: String) {
        self.0.reload.borrow_mut().phase = ReloadPhase::RestoreFailed;
        self.0.reload.borrow_mut().error = Some(error);
        self.0.notify();
    }

    #[cfg(feature = "gpui")]
    pub(crate) fn clear_reload_error(&self) {
        self.0.reload.borrow_mut().phase = ReloadPhase::Idle;
        self.0.reload.borrow_mut().error = None;
        self.0.notify();
    }

    /// Reconstructs an empty replacement workspace. Validates every route/identity
    /// before mounting; mounts are provisional until all checkpoint consumers succeed.
    /// On failure, no partial workspace is attached and the caller retains its checkpoint.
    pub async fn restore_workspace(
        &self,
        checkpoint: &WorkspaceCheckpoint,
        application_id: &str,
    ) -> Result<(), ReloadError> {
        if !self.0.state.borrow().tabs.is_empty()
            || !self.0.state.borrow().tokens.is_empty()
            || self.0.reload.borrow().phase == ReloadPhase::Restoring
            || self.0.quit.borrow().phase.blocks_navigation()
            || self.is_shutdown()
        {
            return Err(ReloadError::Busy);
        }

        self.0.reload.borrow_mut().phase = ReloadPhase::Restoring;
        self.0.tasks.pause();
        self.0.notify();
        let mut guard = RestoreGuard {
            router: self,
            complete: false,
        };

        let result = self.restore_entries(checkpoint, application_id).await;

        if let Err(error) = &result {
            self.0.reload.borrow_mut().phase = ReloadPhase::RestoreFailed;
            self.0.reload.borrow_mut().error = Some(error.to_string());
            self.0.tasks.resume(self.0.tasks.generation(), 0);
            self.0.notify();
        }

        guard.complete = true;

        result
    }

    async fn restore_entries(
        &self,
        checkpoint: &WorkspaceCheckpoint,
        application_id: &str,
    ) -> Result<(), ReloadError> {
        check_size(checkpoint)?;

        if !matches!(checkpoint.version, 1 | 2) || checkpoint.application_id != application_id {
            return Err(ReloadError::Incompatible(
                "workspace version or application identity changed".into(),
            ));
        }

        let generation = GenerationId(
            checkpoint
                .generation
                .0
                .checked_add(1)
                .filter(|_| checkpoint.generation.0 > 0)
                .ok_or_else(|| ReloadError::Incompatible("invalid generation".into()))?,
        );

        let legacy;

        let windows = if checkpoint.version == 1 {
            legacy = vec![crate::reload::WindowCheckpoint {
                id: super::WindowId(1),
                selected: checkpoint.selected,
            }];

            &legacy
        } else {
            &checkpoint.windows
        };

        let mut window_ids = BTreeSet::new();

        if windows
            .iter()
            .any(|window| window.id.0 == 0 || !window_ids.insert(window.id))
            || (windows.len() != 1 && self.0.window_host.borrow().is_none())
        {
            return Err(ReloadError::Incompatible(
                "invalid or unsupported window identities".into(),
            ));
        }

        let mut tab_ids = BTreeSet::new();
        let mut entry_ids = BTreeSet::new();
        let mut sequence = 0;
        let mut decoded = Vec::new();

        for tab in &checkpoint.tabs {
            if !window_ids.contains(&tab.window_id)
                || tab.id.0 == 0
                || !tab_ids.insert(tab.id)
                || tab.entries.is_empty()
                || tab.cursor >= tab.entries.len()
                || self
                    .0
                    .config
                    .history_capacity
                    .is_some_and(|limit| tab.entries.len() > limit)
            {
                return Err(ReloadError::Incompatible(
                    "invalid tab identity/history or changed history capacity".into(),
                ));
            }

            sequence = sequence.max(tab.id.0).max(tab.selected_at);
            let mut destinations = Vec::new();

            for entry in &tab.entries {
                if entry.id.0 == 0
                    || entry.revision > u64::MAX / 2
                    || !entry_ids.insert(entry.id)
                    || (entry.dirty && entry.state.is_none())
                {
                    return Err(ReloadError::Incompatible(
                        "invalid entry identity or missing dirty state".into(),
                    ));
                }

                sequence = sequence.max(entry.id.0);
                let destination = R::decode(&entry.destination)
                    .map_err(|error| ReloadError::Incompatible(error.to_string()))?;

                destination
                    .to_uri()
                    .map_err(|error| ReloadError::Incompatible(error.to_string()))?;
                destinations.push(destination);
            }

            decoded.push(destinations);
        }

        if windows.iter().any(|window| {
            let tabs: Vec<_> = checkpoint
                .tabs
                .iter()
                .filter(|tab| tab.window_id == window.id)
                .collect();

            window.selected.is_none() != tabs.is_empty()
                || window
                    .selected
                    .is_some_and(|id| !tabs.iter().any(|tab| tab.id == id))
        }) || checkpoint.selected.is_some_and(|id| !tab_ids.contains(&id))
            || (checkpoint.version == 1
                && checkpoint.selected.is_none() != checkpoint.tabs.is_empty())
            || sequence > u64::MAX / 2
            || checkpoint.run_sequence > u64::MAX / 2
        {
            return Err(ReloadError::Incompatible(
                "invalid selection or exhausted identity space".into(),
            ));
        }

        sequence = sequence.max(windows.iter().map(|window| window.id.0).max().unwrap_or(1));

        if sequence > u64::MAX / 2 {
            return Err(ReloadError::Incompatible(
                "exhausted window identity space".into(),
            ));
        }

        // Prepare every retained entry before opening any secondary host. A
        // route loader failure must not create a physical window or partially
        // restore the workspace.
        let mut prepared_tabs = Vec::new();
        for (tab, destinations) in checkpoint.tabs.iter().zip(decoded) {
            let scope = self.0.tasks.scope(TaskOwner::Tab(tab.id));
            let mut entries = Vec::new();

            for (saved, destination) in tab.entries.iter().zip(destinations) {
                let entry = Rc::new(Entry {
                    scope: self.0.tasks.scope(TaskOwner::Entry(saved.id)),
                    tab_scope: scope.clone(),
                    id: saved.id,
                    tab: tab.id,
                    window: tab.window_id,
                    destination,
                    revision: Cell::new(saved.revision),
                    dirty: Cell::new(saved.dirty),
                    alive: Cell::new(true),
                    guards: RefCell::new(Vec::new()),
                    resource: RefCell::new(None),
                    cancellation: CancellationToken::new(),
                    reload_capture: RefCell::new(None),
                    restored_state: RefCell::new(saved.state.clone()),
                    state_consumed: Cell::new(false),
                });

                let handle = EntryHandle {
                    core: Rc::downgrade(&self.0),
                    entry: Rc::downgrade(&entry),
                };

                let mut navigator = self.navigator().for_window(tab.window_id);

                navigator.tab = Some(tab.id);
                navigator.origin_entry = Some(saved.id);
                navigator.origin_scope = Rc::downgrade(&entry.scope);

                let loading = (self.0.prepare)(
                    entry.destination.clone(),
                    handle.clone(),
                    navigator.clone(),
                    entry.cancellation.clone(),
                );
                let stopped = navigator.stopped();
                let prepared = match select(Box::pin(loading), Box::pin(stopped)).await {
                    Either::Left((result, _)) => {
                        result.map_err(|error| ReloadError::State(error.to_string()))?
                    }
                    Either::Right(((), _)) => {
                        return Err(ReloadError::State("routing runtime stopped".into()));
                    }
                };

                if self.0.shutdown.get() {
                    return Err(ReloadError::State("routing runtime stopped".into()));
                }

                entries.push((saved.clone(), entry, handle, navigator, prepared));
            }

            prepared_tabs.push((tab.clone(), scope, entries));
        }

        let mut provisional = Vec::new();

        for window in windows {
            if self.0.shutdown.get() {
                return Err(ReloadError::State("routing runtime stopped".into()));
            }
            if window.id != self.0.initial_window.get() {
                let mut host = super::windows::ProvisionalWindow::new(
                    self.0.clone(),
                    window.id,
                    super::OperationId(0),
                );

                host.open(super::Activation::PreserveSelection)
                    .await
                    .map_err(|_| ReloadError::State("cannot create replacement window".into()))?;
                provisional.push(host);
            }
        }

        let mut tabs = Vec::new();

        for (tab, scope, pending_entries) in prepared_tabs {
            let mut entries = Vec::new();

            for (saved, entry, handle, navigator, prepared) in pending_entries {
                let resource =
                    (self.0.prepared_mount)(entry.destination.clone(), handle, navigator, prepared)
                        .await
                        .map_err(|error| ReloadError::State(error.to_string()))?;

                if self.0.shutdown.get() {
                    return Err(ReloadError::State("routing runtime stopped".into()));
                }

                if saved.state.is_some() && !entry.state_consumed.get() {
                    return Err(ReloadError::Incompatible(format!(
                        "entry {} did not consume its reload state",
                        saved.id
                    )));
                }

                entry.dirty.set(saved.dirty);
                entry.revision.set(saved.revision);
                *entry.resource.borrow_mut() = Some(resource);
                entries.push(entry);
            }

            tabs.push(Tab {
                scope,
                id: tab.id,
                window: tab.window_id,
                entries,
                cursor: tab.cursor,
                mode: tab.mode,
                selected_at: tab.selected_at,
            });
        }

        let mut state = self.0.state.borrow_mut();

        let retired: Vec<_> = state
            .windows
            .keys()
            .filter(|id| !window_ids.contains(id))
            .copied()
            .collect();

        state.windows = windows
            .iter()
            .map(|window| {
                (
                    window.id,
                    super::WindowState {
                        selected: window.selected,
                        visibility: state
                            .windows
                            .get(&window.id)
                            .map_or(HostVisibility::Unknown, |host| host.visibility),
                        ..super::WindowState::default()
                    },
                )
            })
            .collect();
        state.tabs = tabs;

        for host in &mut provisional {
            host.committed = true;
        }

        state.sequence = sequence;

        if !window_ids.contains(&self.0.initial_window.get())
            && let Some(first) = windows.first()
        {
            self.0.initial_window.set(first.id);
        }

        drop(state);
        let host = self.0.window_host.borrow().clone();

        for window in retired {
            if let Some(host) = &host {
                host.close(window);
            }

            self.0.tasks.forget_window(window);

            #[cfg(feature = "gpui")]
            {
                self.0.host.borrow_mut().remove(&window);

                if let Some(contexts) = self.0.view_contexts.borrow().as_ref() {
                    contexts.borrow_mut().remove(&window);
                }
            }
        }

        *self.0.reload.borrow_mut() = ReloadStatus {
            restored_from: Some(checkpoint.source_runtime),
            generation,
            phase: ReloadPhase::Restored,
            request: None,
            error: None,
        };
        self.0.tasks.resume(generation, checkpoint.run_sequence);
        {
            let state = self.0.state.borrow();

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
        self.0.emit_history(true);
        self.0.notify();

        Ok(())
    }
}

struct RestoreGuard<'a, R: Route> {
    router: &'a Router<R>,
    complete: bool,
}

impl<R: Route> Drop for RestoreGuard<'_, R> {
    fn drop(&mut self) {
        if !self.complete {
            self.router.0.reload.borrow_mut().phase = ReloadPhase::RestoreFailed;
            self.router.0.reload.borrow_mut().error = Some("restoration was interrupted".into());
            self.router
                .0
                .tasks
                .resume(self.router.0.tasks.generation(), 0);
            self.router.0.notify();
        }
    }
}

fn check_size(checkpoint: &WorkspaceCheckpoint) -> Result<(), ReloadError> {
    if serde_json::to_vec(checkpoint)
        .map_err(|error| ReloadError::State(error.to_string()))?
        .len()
        > MAX_CHECKPOINT_BYTES
    {
        return Err(ReloadError::TooLarge);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    //! Covers: provisional mounts, checkpoint consumers and recreated activity policies.
    //! Real: local mount adapter and runtime; Doubles: unit resources in place of GPUI views.
    //! Requires: no display or platform services.
    use super::*;
    use crate::diagnostics::Collector;
    use crate::reload::{EntryCheckpoint, WindowCheckpoint};
    use crate::router::{
        EntryId, GenerationId, MountError, OpenOptions, Outcome, RouterConfig, RuntimeId, TabId,
        TabMode, WindowHost, WindowId,
    };
    use futures::executor::{LocalPool, block_on};
    use futures::task::LocalSpawnExt;

    #[derive(Clone, crate::Route)]
    enum Destination {
        #[route(id = "page", path = "/page/{id}")]
        Page(u64),
    }

    #[crate::data]
    struct Draft {
        edits: u64,
    }

    impl ReloadState for Draft {
        const SCHEMA: &'static str = "draft";
        const VERSION: u32 = 1;
    }

    #[test]
    fn entry_state_is_validated_before_attach_and_factories_restart_only_when_eligible() {
        let source = Router::new(RouterConfig::default());
        let mut pool = LocalPool::new();
        let driver = source.clone();

        pool.spawner()
            .spawn_local(async move {
                driver.run().await.unwrap();
            })
            .unwrap();

        for id in 1..=2 {
            let operation = source
                .navigator()
                .open_with(Destination::Page(id), OpenOptions::new_tab());

            let Outcome::Committed(opened) = pool.run_until(operation.wait()).unwrap() else {
                panic!()
            };

            let entry = source.entry(opened.entry_id).unwrap();

            entry
                .on_reload(move || Draft { edits: id * 10 }.checkpoint())
                .unwrap();
            entry.set_dirty(true).unwrap();
        }

        source
            .prepare_reload(ReloadRequest {
                id: "test".into(),
                mechanism: crate::reload::ReloadMechanism::NativeRestart,
            })
            .unwrap();
        pool.run_until_stalled();
        let checkpoint = source.reload_checkpoint("test").unwrap();
        let runs = Rc::new(RefCell::new(Vec::new()));
        let observed = runs.clone();
        let mounted = Rc::new(RefCell::new(Vec::new()));
        let states = mounted.clone();

        let mount = Rc::new(move |_: Destination, entry: EntryHandle<Destination>, _| {
            let state = entry.reload_state::<Draft>();
            let runs = observed.clone();
            let mounted = states.clone();

            Box::pin(async move {
                let draft = state
                    .map_err(|error| crate::router::MountError(error.to_string()))?
                    .unwrap();

                mounted.borrow_mut().push(draft.edits);
                entry.on_reload(move || draft.checkpoint()).unwrap();
                entry
                    .activity("refresh", move |run| {
                        runs.borrow_mut().push(run.id());

                        async {
                            futures::future::pending::<()>().await;

                            Ok(())
                        }
                    })
                    .unwrap();

                Ok(Rc::new(()) as super::super::Resource)
            }) as futures::future::LocalBoxFuture<'static, _>
        });

        let replacement = Router::with_mount(RouterConfig::default(), Collector::default(), mount);

        block_on(replacement.restore_workspace(&checkpoint, "test")).unwrap();
        let driven = replacement.clone();

        pool.spawner()
            .spawn_local(async move {
                driven.run().await.unwrap();
            })
            .unwrap();
        pool.run_until_stalled();
        assert_eq!(&*mounted.borrow(), &[10, 20]);
        assert_eq!(
            runs.borrow().len(),
            1,
            "hidden Foreground factory stays suspended"
        );
        assert!(
            replacement
                .snapshot()
                .tabs
                .iter()
                .all(|tab| tab.entries[0].dirty)
        );
        let mut bad = checkpoint.clone();

        bad.tabs[1].entries[0].state.as_mut().unwrap().version = 999;
        let failed_mount = Rc::new(|_: Destination, entry: EntryHandle<Destination>, _| {
            let state = entry.reload_state::<Draft>();

            Box::pin(async move {
                state.map_err(|error| crate::router::MountError(error.to_string()))?;

                Ok(Rc::new(()) as super::super::Resource)
            }) as futures::future::LocalBoxFuture<'static, _>
        });

        let failed =
            Router::with_mount(RouterConfig::default(), Collector::default(), failed_mount);

        assert!(block_on(failed.restore_workspace(&bad, "test")).is_err());
        assert!(
            failed.snapshot().tabs.is_empty(),
            "earlier provisional mounts must not attach"
        );
    }

    #[test]
    fn loaded_checkpoint_failure_happens_before_secondary_host_creation() {
        struct Host(Rc<Cell<usize>>);

        impl WindowHost for Host {
            fn open(
                &self,
                _: WindowId,
                _: super::super::Activation,
            ) -> futures::future::LocalBoxFuture<'static, Result<(), MountError>> {
                self.0.set(self.0.get() + 1);
                Box::pin(async { Ok(()) })
            }

            fn close(&self, _: WindowId) {}

            fn activate(&self, _: WindowId) {}
        }

        let opened = Rc::new(Cell::new(0));
        let prepare: super::super::Prepare<Destination> = Rc::new(|_, _, _, _| {
            Box::pin(async { Err(MountError("checkpoint loader failed".into())) })
        });
        let mount: super::super::PreparedMount<Destination> = Rc::new(|_, _, _, prepared| {
            Box::pin(async move {
                prepared.downcast::<()>()?;
                Ok(Rc::new(()) as super::super::Resource)
            })
        });
        let router = Router::with_prepared_mount(
            RouterConfig::default(),
            Collector::default(),
            prepare,
            mount,
        );
        router.window_host(Host(opened.clone())).unwrap();

        let tab = TabId(2);
        let checkpoint = WorkspaceCheckpoint {
            windows: vec![
                WindowCheckpoint {
                    id: WindowId(1),
                    selected: None,
                },
                WindowCheckpoint {
                    id: WindowId(2),
                    selected: Some(tab),
                },
            ],
            version: 2,
            application_id: "app".into(),
            source_runtime: RuntimeId(uuid::Uuid::new_v4()),
            generation: GenerationId(1),
            run_sequence: 0,
            selected: Some(tab),
            tabs: vec![TabCheckpoint {
                window_id: WindowId(2),
                id: tab,
                mode: TabMode::Foreground,
                cursor: 0,
                selected_at: 0,
                entries: vec![EntryCheckpoint {
                    id: EntryId(2),
                    destination: Destination::Page(7).encode(),
                    dirty: false,
                    revision: 0,
                    state: None,
                }],
            }],
        };

        assert!(block_on(router.restore_workspace(&checkpoint, "app")).is_err());
        assert_eq!(opened.get(), 0);
        assert!(router.snapshot().tabs.is_empty());

        // A ready loader can shut down its runtime in the same poll. The
        // shutdown branch of select may not win, so revalidate before hosts.
        let stop = Rc::new(RefCell::new(
            None::<std::rc::Weak<super::super::Core<Destination>>>,
        ));
        let signal = stop.clone();
        let prepare = Rc::new(move |_, _, _, _| {
            let signal = signal.clone();
            Box::pin(async move {
                if let Some(core) = signal.borrow().as_ref().and_then(std::rc::Weak::upgrade) {
                    Router(core).shutdown();
                }
                Ok(super::super::PreparedPage::from_value(()))
            }) as futures::future::LocalBoxFuture<'static, _>
        });
        let mount = Rc::new(|_, _, _, _| {
            Box::pin(async { Ok(Rc::new(()) as super::super::Resource) })
                as futures::future::LocalBoxFuture<'static, _>
        });
        let stopped = Router::with_prepared_mount(
            RouterConfig::default(),
            Collector::default(),
            prepare,
            mount,
        );
        *stop.borrow_mut() = Some(Rc::downgrade(&stopped.0));
        stopped.window_host(Host(opened.clone())).unwrap();
        assert!(block_on(stopped.restore_workspace(&checkpoint, "app")).is_err());
        assert_eq!(opened.get(), 0);
        assert!(stopped.snapshot().tabs.is_empty());
    }
}
