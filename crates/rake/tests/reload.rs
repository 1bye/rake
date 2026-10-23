//! Covers: compatible workspace restoration, boundaries, generations and explicit failures.
//! Real: router, managed-work driver, checkpoint codec and public control API.
//! Doubles: a history sink and logical entry-state provider; no native code replacement.
//! Requires: no GPU, browser, network or sleeps.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use futures::executor::LocalPool;
use futures::task::LocalSpawnExt;
use rake::prelude::*;
use rake::reload::{ReloadMechanism, ReloadPhase, ReloadRequest};
use rake::router::{HistoryChange, Operation, OperationError};

#[derive(Clone, Route)]
enum Destination {
    #[route(id = "page", path = "/page/{id}")]
    Page(u64),
}

#[rake::data]
struct Draft {
    edits: u64,
}

impl ReloadState for Draft {
    const SCHEMA: &'static str = "test.draft";
    const VERSION: u32 = 2;
}

fn harness() -> (Router<Destination>, LocalPool) {
    let router = Router::new(RouterConfig::default());
    let pool = LocalPool::new();
    let driver = router.clone();

    pool.spawner()
        .spawn_local(async move {
            driver.run().await.unwrap();
        })
        .unwrap();

    (router, pool)
}

fn commit<T: Clone>(pool: &mut LocalPool, operation: Operation<T>) -> T {
    match pool.run_until(operation.wait()).unwrap() {
        Outcome::Committed(value) => value,
        _ => panic!("commit expected"),
    }
}

fn prepare(router: &Router<Destination>, pool: &mut LocalPool) {
    router
        .prepare_reload(ReloadRequest {
            id: "build-2".into(),
            mechanism: ReloadMechanism::NativeCodeReplacement,
        })
        .unwrap();
    pool.run_until_stalled();
    assert_eq!(router.reload_status().phase, ReloadPhase::Ready);
}

#[test]
fn restores_independent_tabs_histories_selection_modes_and_only_replaces_browser_visit() {
    let (old, mut pool) = harness();

    let a = commit(
        &mut pool,
        old.navigator().open_with(
            Destination::Page(1),
            OpenOptions::new_tab().mode(TabMode::Live),
        ),
    );

    commit(
        &mut pool,
        old.navigator().tab(a.tab_id).navigate(Destination::Page(2)),
    );
    commit(&mut pool, old.navigator().tab(a.tab_id).back());
    commit(
        &mut pool,
        old.navigator().open_with(
            Destination::Page(3),
            OpenOptions::new_tab().activation(Activation::PreserveSelection),
        ),
    );
    prepare(&old, &mut pool);
    let checkpoint = old.reload_checkpoint("test").unwrap();
    let replacement = Router::<Destination>::new(RouterConfig::default());
    let visits = Rc::new(RefCell::new(Vec::new()));
    let history = visits.clone();

    replacement.on_history(move |change| {
        history.borrow_mut().push(change);

        Ok(())
    });
    pool.run_until(replacement.restore_workspace(&checkpoint, "test"))
        .unwrap();
    assert_ne!(replacement.runtime_id(), old.runtime_id());
    assert_eq!(replacement.snapshot().selected, Some(a.tab_id));
    assert_eq!(replacement.snapshot().tabs[0].mode, TabMode::Live);
    assert_eq!(replacement.snapshot().tabs[0].cursor, 0);
    assert_eq!(
        replacement.snapshot().tabs[0].entries[1].destination,
        Destination::Page(2).encode()
    );
    assert_eq!(replacement.snapshot().tabs[0].entries[0].id, a.entry_id);
    assert_eq!(visits.borrow().len(), 1);
    assert!(matches!(visits.borrow()[0], HistoryChange::Replace(..)));
    assert_ne!(
        replacement.reload_status().generation,
        checkpoint.generation
    );
}

#[test]
fn work_is_dropped_before_ready_and_stale_runs_never_publish_after_apply() {
    let (router, mut pool) = harness();

    let opened = commit(
        &mut pool,
        router
            .navigator()
            .open(Destination::Page(1), OpenTarget::NewTab),
    );

    let entry = router.entry(opened.entry_id).unwrap();
    let runs = Rc::new(RefCell::new(Vec::new()));
    let observed = runs.clone();
    let drops = Rc::new(Cell::new(0));
    let dropped = drops.clone();

    struct OnDrop(Rc<Cell<u64>>);

    impl Drop for OnDrop {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    entry.on_event(|_: u64| Ok(())).unwrap();
    let handle = entry
        .activity("refresh", move |run| {
            observed.borrow_mut().push(run);
            let guard = OnDrop(dropped.clone());

            async move {
                let _guard = guard;

                futures::future::pending::<()>().await;

                Ok(())
            }
        })
        .unwrap();

    pool.run_until_stalled();
    let one_shot = router
        .app()
        .tasks()
        .spawn("one shot", async {
            futures::future::pending::<()>().await;

            Ok(())
        })
        .unwrap();

    pool.run_until_stalled();
    let before = handle.snapshot().unwrap();
    let old_run = runs.borrow()[0].clone();

    router
        .prepare_reload(ReloadRequest {
            id: "swap".into(),
            mechanism: ReloadMechanism::NativeCodeReplacement,
        })
        .unwrap();
    assert_eq!(router.reload_status().phase, ReloadPhase::Quiescing);
    assert!(old_run.publish(1u64).is_err());
    assert!(matches!(
        router.app().tasks().spawn("late", async { Ok(()) }),
        Err(TaskError::Reloading)
    ));
    assert!(matches!(
        pool.run_until(
            router
                .navigator()
                .open(Destination::Page(2), OpenTarget::NewTab)
                .wait()
        ),
        Err(OperationError::Reloading)
    ));
    pool.run_until_stalled();
    assert_eq!(drops.get(), 1);
    assert_eq!(router.reload_status().phase, ReloadPhase::Ready);
    let runtime = router.runtime_id();

    router.finish_reload("swap", true).unwrap();
    pool.run_until_stalled();
    assert_eq!(router.runtime_id(), runtime);
    assert_ne!(
        handle.snapshot().unwrap().generation_id,
        before.generation_id
    );
    assert_ne!(handle.snapshot().unwrap().run_id, before.run_id);
    assert_eq!(runs.borrow().len(), 2);
    assert_eq!(
        one_shot.snapshot().unwrap().state,
        rake::tasks::TaskState::Cancelled
    );
    assert!(old_run.publish(2u64).is_err());
}

#[test]
fn dirty_state_requires_opt_in_and_schema_migrations_are_explicit() {
    let (router, mut pool) = harness();

    let opened = commit(
        &mut pool,
        router
            .navigator()
            .open(Destination::Page(1), OpenTarget::NewTab),
    );

    let entry = router.entry(opened.entry_id).unwrap();

    entry.set_dirty(true).unwrap();
    prepare(&router, &mut pool);
    assert!(matches!(
        router.reload_checkpoint("test"),
        Err(ReloadError::MissingState(_))
    ));
    entry
        .on_reload(|| Draft { edits: 42 }.checkpoint())
        .unwrap();
    let checkpoint = router.reload_checkpoint("test").unwrap();
    let saved = checkpoint.tabs[0].entries[0].state.clone().unwrap();

    assert_eq!(Draft::restore(saved.clone()).unwrap().edits, 42);
    let mut incompatible = saved;

    incompatible.version = 99;
    assert!(matches!(
        Draft::restore(incompatible),
        Err(ReloadError::Incompatible(_))
    ));
    assert!(router.finish_reload("wrong-build", false).is_err());
    router.finish_reload("build-2", false).unwrap();
    assert_eq!(router.reload_status().phase, ReloadPhase::Aborted);
    assert!(entry.is_dirty().unwrap());
}

#[test]
fn incompatible_restore_does_not_attach_partial_state_and_can_retry() {
    let (router, mut pool) = harness();

    commit(
        &mut pool,
        router
            .navigator()
            .open(Destination::Page(1), OpenTarget::NewTab),
    );
    prepare(&router, &mut pool);
    let checkpoint = router.reload_checkpoint("test").unwrap();
    let replacement = Router::<Destination>::new(RouterConfig::default());
    let mut bad = checkpoint.clone();

    bad.tabs[0].entries[0].destination.route = "removed-route".into();
    assert!(
        pool.run_until(replacement.restore_workspace(&bad, "test"))
            .is_err()
    );
    assert!(replacement.snapshot().tabs.is_empty());
    assert_eq!(
        replacement.reload_status().phase,
        ReloadPhase::RestoreFailed
    );
    pool.run_until(replacement.restore_workspace(&checkpoint, "test"))
        .unwrap();
    assert_eq!(replacement.snapshot().tabs.len(), 1);
    assert!(
        pool.run_until(replacement.restore_workspace(&checkpoint, "test"))
            .is_err()
    );
}

#[test]
fn pending_navigation_blocks_reload_instead_of_cancelling_an_operation() {
    let (router, mut pool) = harness();

    let pending = router
        .navigator()
        .open(Destination::Page(1), OpenTarget::NewTab);

    assert!(matches!(
        router.prepare_reload(ReloadRequest {
            id: "busy".into(),
            mechanism: ReloadMechanism::NativeRestart
        }),
        Err(ReloadError::Busy)
    ));
    commit(&mut pool, pending);
    assert_eq!(router.snapshot().tabs.len(), 1);
}
