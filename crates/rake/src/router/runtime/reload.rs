//! Owned development state and the quiescent reload boundary.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;

use crate::http::CancellationToken;

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
    /// Current reload state. Ready means managed futures have been dropped and the
    /// host UI has acknowledged detachment; it does not prove native code unload safety.
    pub fn reload_status(&self) -> ReloadStatus {
        let mut status = self.0.reload.borrow().clone();
        let hidden = self.0.tasks.snapshot().host_visibility == HostVisibility::Hidden;

        if status.phase == ReloadPhase::Quiescing
            && self.0.tasks.quiescent()
            && (self.0.reload_ui_ready.get() || hidden)
        {
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

        if !state.tokens.is_empty()
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
        self.0.reload_ui_ready.set(!self.0.has_window.get());
        self.0.tasks.pause();
        self.0.notify();

        Ok(self.reload_status())
    }

    #[cfg(feature = "gpui")]
    pub(crate) fn acknowledge_reload_ui(&self) {
        if !self.0.reload_ui_ready.replace(true) {
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
                    tab.mode,
                    tab.cursor,
                    tab.selected_at,
                    tab.entries.clone(),
                )
            })
            .collect();

        let mut checkpoint_tabs = Vec::new();

        for (id, mode, cursor, selected_at, entries) in tabs {
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
            version: 1,
            application_id: application_id.into(),
            source_runtime: self.runtime_id(),
            generation: self.0.tasks.generation(),
            run_sequence: self.0.tasks.run_sequence(),
            selected: state.selected,
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

        if checkpoint.version != 1 || checkpoint.application_id != application_id {
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

        let mut tab_ids = BTreeSet::new();
        let mut entry_ids = BTreeSet::new();
        let mut sequence = 0;
        let mut decoded = Vec::new();

        for tab in &checkpoint.tabs {
            if tab.id.0 == 0
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

        if checkpoint.selected.is_some_and(|id| !tab_ids.contains(&id))
            || (checkpoint.selected.is_none() != checkpoint.tabs.is_empty())
            || sequence > u64::MAX / 2
            || checkpoint.run_sequence > u64::MAX / 2
        {
            return Err(ReloadError::Incompatible(
                "invalid selection or exhausted identity space".into(),
            ));
        }

        let mut tabs = Vec::new();

        for (tab, destinations) in checkpoint.tabs.iter().zip(decoded) {
            let scope = self.0.tasks.scope(TaskOwner::Tab(tab.id));
            let mut entries = Vec::new();

            for (saved, destination) in tab.entries.iter().zip(destinations) {
                let entry = Rc::new(Entry {
                    scope: self.0.tasks.scope(TaskOwner::Entry(saved.id)),
                    tab_scope: scope.clone(),
                    id: saved.id,
                    tab: tab.id,
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

                let mut navigator = self.navigator();

                navigator.tab = Some(tab.id);
                navigator.origin_entry = Some(saved.id);
                navigator.origin_scope = Rc::downgrade(&entry.scope);
                let resource = (self.0.mount)(entry.destination.clone(), handle, navigator)
                    .await
                    .map_err(|error| ReloadError::State(error.to_string()))?;

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
                entries,
                cursor: tab.cursor,
                mode: tab.mode,
                selected_at: tab.selected_at,
            });
        }

        let mut state = self.0.state.borrow_mut();

        state.tabs = tabs;
        state.selected = checkpoint.selected;
        state.sequence = sequence;
        drop(state);
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
                        current && state.selected == Some(tab.id),
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
    use crate::router::{OpenOptions, Outcome, RouterConfig};
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
}
