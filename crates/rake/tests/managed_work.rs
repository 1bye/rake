//! Covers: scope lifetimes, activity eligibility, fresh runs and stale publication rejection.
//! Real: router, local work driver, factories, cancellation, typed dispatch and retention.
//! Doubles: explicit host visibility signals; platform adapters need host smoke checks.
//! Requires: no display, GPU, network or arbitrary sleeps.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use futures::FutureExt;
use futures::channel::oneshot;
use futures::executor::LocalPool;
use futures::task::LocalSpawnExt;
use rake::prelude::*;
use rake::router::{EntryHandle, Opened, Operation, RouterConfig};
use rake::tasks::{HostVisibility, TaskConfig, TaskKind, TaskOwner, TaskState};

#[derive(Clone, Route)]
enum Destination {
    #[route(id = "page", path = "/page/{id}")]
    Page(u64),
}

fn harness(config: RouterConfig) -> (Router<Destination>, LocalPool) {
    let router = Router::new(config);
    let pool = LocalPool::new();
    let driver = router.clone();

    pool.spawner()
        .spawn_local(async move {
            driver.run().await.expect("driver");
        })
        .expect("spawn");

    (router, pool)
}

fn commit<T: Clone>(pool: &mut LocalPool, operation: Operation<T>) -> T {
    match pool.run_until(operation.wait()).expect("operation") {
        Outcome::Committed(result) => result,
        _ => panic!("expected commit"),
    }
}

fn open(router: &Router<Destination>, pool: &mut LocalPool, id: u64, mode: TabMode) -> Opened {
    commit(
        pool,
        router
            .navigator()
            .open_with(Destination::Page(id), OpenOptions::new_tab().mode(mode)),
    )
}

struct Dropped(Rc<Cell<usize>>);

impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

fn activity(
    entry: &EntryHandle<Destination>,
    visible: bool,
    runs: Rc<RefCell<Vec<ActivityRun>>>,
    drops: Rc<Cell<usize>>,
) -> TaskHandle {
    let factory = move |run| {
        runs.borrow_mut().push(run);
        let dropped = Dropped(drops.clone());

        async move {
            let _dropped = dropped;

            futures::future::pending::<()>().await;

            Ok(())
        }
    };

    if visible {
        entry.while_visible("visible", factory)
    } else {
        entry.activity("refresh", factory)
    }
    .expect("activity")
}

#[test]
fn foreground_hiding_cancels_and_returning_constructs_a_fresh_local_future() {
    let (router, mut pool) = harness(RouterConfig::default());
    let a = open(&router, &mut pool, 1, TabMode::Foreground);
    let runs = Rc::new(RefCell::new(Vec::new()));
    let drops = Rc::new(Cell::new(0));

    let handle = activity(
        &router.entry(a.entry_id).unwrap(),
        false,
        runs.clone(),
        drops.clone(),
    );

    pool.run_until_stalled();
    let first = runs.borrow()[0].clone();

    open(&router, &mut pool, 2, TabMode::Foreground);
    pool.run_until_stalled();
    assert!(first.cancellation().is_cancelled());
    assert_eq!(drops.get(), 1);
    assert_eq!(handle.snapshot().unwrap().state, TaskState::Cancelled);
    commit(&mut pool, router.navigator().tab(a.tab_id).select());
    pool.run_until_stalled();
    assert_eq!(runs.borrow().len(), 2);
    assert_ne!(first.id(), runs.borrow()[1].id());
    assert!(!runs.borrow()[1].cancellation().is_cancelled());
}

#[test]
fn live_only_keeps_current_entry_activity_and_visibility_activity_still_stops() {
    let (router, mut pool) = harness(RouterConfig::default());
    let a = open(&router, &mut pool, 1, TabMode::Live);
    let runs = Rc::new(RefCell::new(Vec::new()));
    let visible = Rc::new(RefCell::new(Vec::new()));

    activity(
        &router.entry(a.entry_id).unwrap(),
        false,
        runs.clone(),
        Rc::new(Cell::new(0)),
    );
    activity(
        &router.entry(a.entry_id).unwrap(),
        true,
        visible.clone(),
        Rc::new(Cell::new(0)),
    );
    pool.run_until_stalled();
    open(&router, &mut pool, 2, TabMode::Foreground);
    pool.run_until_stalled();
    assert!(!runs.borrow()[0].cancellation().is_cancelled());
    assert!(visible.borrow()[0].cancellation().is_cancelled());
    let tab = router.navigator().tab(a.tab_id);

    commit(&mut pool, tab.navigate(Destination::Page(3)));
    pool.run_until_stalled();
    assert!(runs.borrow()[0].cancellation().is_cancelled());
    commit(&mut pool, tab.back());
    pool.run_until_stalled();
    assert_eq!(runs.borrow().len(), 2);
    assert_eq!(
        visible.borrow().len(),
        1,
        "hidden historical visibility factory stays stopped"
    );
}

#[test]
fn policy_changes_and_host_visibility_recompute_eligibility_without_duplicate_runs() {
    let (router, mut pool) = harness(RouterConfig::default());
    let a = open(&router, &mut pool, 1, TabMode::Foreground);
    let runs = Rc::new(RefCell::new(Vec::new()));

    activity(
        &router.entry(a.entry_id).unwrap(),
        false,
        runs.clone(),
        Rc::new(Cell::new(0)),
    );
    pool.run_until_stalled();
    router.set_host_visibility(HostVisibility::Hidden);
    assert!(runs.borrow()[0].cancellation().is_cancelled());
    commit(
        &mut pool,
        router.navigator().tab(a.tab_id).set_mode(TabMode::Live),
    );
    pool.run_until_stalled();
    assert_eq!(runs.borrow().len(), 2);
    router.set_host_visibility(HostVisibility::Visible);
    pool.run_until_stalled();
    assert_eq!(
        runs.borrow().len(),
        2,
        "eligible Live run continues across visibility changes"
    );
    commit(
        &mut pool,
        router
            .navigator()
            .tab(a.tab_id)
            .set_mode(TabMode::Foreground),
    );
    pool.run_until_stalled();
    assert_eq!(
        runs.borrow().len(),
        2,
        "visible mode change must not restart"
    );
    router.set_host_visibility(HostVisibility::Hidden);
    router.set_host_visibility(HostVisibility::Visible);
    pool.run_until_stalled();
    assert_eq!(runs.borrow().len(), 3);
    assert_eq!(
        router.snapshot().work.host_visibility,
        HostVisibility::Visible
    );
}

#[test]
fn tab_tasks_survive_navigation_and_selection_but_application_tasks_survive_tab_close() {
    let (router, mut pool) = harness(RouterConfig::default());
    let a = open(&router, &mut pool, 1, TabMode::Foreground);
    let tab = router.navigator().tab(a.tab_id);
    let tab_drops = Rc::new(Cell::new(0));
    let app_drops = Rc::new(Cell::new(0));
    let tab_guard = Dropped(tab_drops.clone());
    let app_guard = Dropped(app_drops.clone());

    let task = tab
        .tasks()
        .spawn("download", async move {
            let _guard = tab_guard;

            futures::future::pending::<()>().await;

            Ok(())
        })
        .unwrap();

    let app = router
        .app()
        .tasks()
        .spawn("export", async move {
            let _guard = app_guard;

            futures::future::pending::<()>().await;

            Ok(())
        })
        .unwrap();

    pool.run_until_stalled();
    commit(&mut pool, tab.navigate(Destination::Page(2)));
    open(&router, &mut pool, 3, TabMode::Foreground);
    pool.run_until_stalled();
    assert_eq!(task.snapshot().unwrap().state, TaskState::Running);
    commit(&mut pool, tab.close());
    pool.run_until_stalled();
    assert_eq!(task.snapshot().unwrap().state, TaskState::Cancelled);
    assert_eq!(tab_drops.get(), 1);
    assert_eq!(app.snapshot().unwrap().state, TaskState::Running);
    assert_eq!(app_drops.get(), 0);
    assert!(matches!(
        tab.tasks().spawn("closed", async { Ok(()) }),
        Err(TaskError::OwnerDisposed)
    ));
    router.shutdown();
    pool.run_until_stalled();
    assert_eq!(app_drops.get(), 1);
    assert_eq!(app.snapshot().unwrap().state, TaskState::Cancelled);
}

#[test]
fn finite_entry_tasks_continue_when_hidden_and_revisited_from_history() {
    let (router, mut pool) = harness(RouterConfig::default());
    let a = open(&router, &mut pool, 1, TabMode::Foreground);
    let entry = router.entry(a.entry_id).unwrap();
    let (release, wait) = oneshot::channel();

    let task = entry
        .tasks()
        .unwrap()
        .spawn("entry-load", async move {
            wait.await.expect("release entry load");
            Ok(())
        })
        .unwrap();
    let tab = router.navigator().tab(a.tab_id);

    pool.run_until_stalled();
    assert_eq!(task.snapshot().unwrap().state, TaskState::Running);
    commit(&mut pool, tab.navigate(Destination::Page(2)));
    pool.run_until_stalled();
    assert_eq!(task.snapshot().unwrap().state, TaskState::Running);
    commit(&mut pool, tab.back());
    pool.run_until_stalled();
    assert_eq!(task.snapshot().unwrap().state, TaskState::Running);

    release.send(()).unwrap();
    pool.run_until_stalled();
    assert_eq!(task.snapshot().unwrap().state, TaskState::Completed);
}

#[test]
fn finite_entry_tasks_cancel_on_truncation_or_close_while_app_work_survives() {
    let (router, mut pool) = harness(RouterConfig::default());
    let a = open(&router, &mut pool, 1, TabMode::Foreground);
    let tab = router.navigator().tab(a.tab_id);
    let b = commit(&mut pool, tab.navigate(Destination::Page(2)));
    let b_entry = router.entry(b.entry_id).unwrap();
    let b_drops = Rc::new(Cell::new(0));
    let b_guard = Dropped(b_drops.clone());
    let b_task = b_entry
        .tasks()
        .unwrap()
        .spawn("truncated-entry", async move {
            let _guard = b_guard;
            futures::future::pending::<()>().await;
            Ok(())
        })
        .unwrap();

    let app_drops = Rc::new(Cell::new(0));
    let app_guard = Dropped(app_drops.clone());
    let app_task = router
        .app()
        .tasks()
        .spawn("application-export", async move {
            let _guard = app_guard;
            futures::future::pending::<()>().await;
            Ok(())
        })
        .unwrap();

    pool.run_until_stalled();
    commit(&mut pool, tab.back());
    let c = commit(&mut pool, tab.navigate(Destination::Page(3)));
    pool.run_until_stalled();
    assert_eq!(b_task.snapshot().unwrap().state, TaskState::Cancelled);
    assert_eq!(b_drops.get(), 1);
    assert_eq!(app_task.snapshot().unwrap().state, TaskState::Running);
    assert_eq!(app_drops.get(), 0);

    let c_entry = router.entry(c.entry_id).unwrap();
    let c_drops = Rc::new(Cell::new(0));
    let c_guard = Dropped(c_drops.clone());
    let c_task = c_entry
        .tasks()
        .unwrap()
        .spawn("closed-entry", async move {
            let _guard = c_guard;
            futures::future::pending::<()>().await;
            Ok(())
        })
        .unwrap();
    pool.run_until_stalled();
    commit(&mut pool, tab.close());
    pool.run_until_stalled();

    assert_eq!(c_task.snapshot().unwrap().state, TaskState::Cancelled);
    assert_eq!(c_drops.get(), 1);
    assert_eq!(app_task.snapshot().unwrap().state, TaskState::Running);
    assert_eq!(app_drops.get(), 0);

    router.shutdown();
    pool.run_until_stalled();
    assert_eq!(app_task.snapshot().unwrap().state, TaskState::Cancelled);
    assert_eq!(app_drops.get(), 1);
}

#[test]
fn activity_publication_rejects_after_cancel_and_entry_close() {
    let (router, mut pool) = harness(RouterConfig::default());
    let a = open(&router, &mut pool, 1, TabMode::Foreground);
    let entry = router.entry(a.entry_id).unwrap();
    entry.on_event(|_: u32| Ok(())).unwrap();
    let runs = Rc::new(RefCell::new(Vec::new()));
    let observed = runs.clone();
    let task = entry
        .activity("cancelled", move |run| {
            observed.borrow_mut().push(run.clone());
            async move {
                futures::future::pending::<()>().await;
                Ok(())
            }
        })
        .unwrap();

    pool.run_until_stalled();
    let cancelled = runs.borrow()[0].clone();
    assert!(task.cancel().unwrap());
    pool.run_until_stalled();
    assert_eq!(cancelled.publish(1_u32), Err(TaskError::StaleRun));

    let tab = router.navigator().tab(a.tab_id);
    let b = commit(&mut pool, tab.navigate(Destination::Page(2)));
    let b_entry = router.entry(b.entry_id).unwrap();
    b_entry.on_event(|_: u32| Ok(())).unwrap();
    let closed_runs = Rc::new(RefCell::new(Vec::new()));
    let observed = closed_runs.clone();
    b_entry
        .activity("closed", move |run| {
            observed.borrow_mut().push(run.clone());
            async move {
                futures::future::pending::<()>().await;
                Ok(())
            }
        })
        .unwrap();
    pool.run_until_stalled();
    let closed = closed_runs.borrow()[0].clone();

    commit(&mut pool, tab.close());
    pool.run_until_stalled();
    assert_eq!(closed.publish(2_u32), Err(TaskError::OwnerDisposed));
}

#[test]
fn old_and_queued_activity_results_cannot_overwrite_a_new_run() {
    let (router, mut pool) = harness(RouterConfig::default());
    let a = open(&router, &mut pool, 1, TabMode::Foreground);
    let entry = router.entry(a.entry_id).unwrap();
    let values = Rc::new(RefCell::new(Vec::new()));
    let applied = values.clone();

    entry
        .on_event(move |value: u64| {
            applied.borrow_mut().push(value);

            Ok(())
        })
        .unwrap();
    let runs = Rc::new(RefCell::new(Vec::new()));

    activity(&entry, false, runs.clone(), Rc::new(Cell::new(0)));
    pool.run_until_stalled();
    let old = runs.borrow()[0].clone();

    old.publish(1_u64).unwrap();
    // Invalidate before allowing the driver's queued handler to run.
    router.set_host_visibility(HostVisibility::Hidden);
    pool.run_until_stalled();
    assert!(values.borrow().is_empty());
    assert_eq!(old.publish(2_u64), Err(TaskError::StaleRun));
    router.set_host_visibility(HostVisibility::Visible);
    pool.run_until_stalled();
    let current = runs.borrow()[1].clone();

    current.publish(3_u64).unwrap();
    pool.run_until_stalled();
    assert_eq!(*values.borrow(), vec![3]);
    commit(&mut pool, router.navigator().tab(a.tab_id).close());
    assert_eq!(current.publish(4_u64), Err(TaskError::OwnerDisposed));
}

#[test]
fn successful_completion_delivers_its_queued_event_once_without_restarting() {
    let (router, mut pool) = harness(RouterConfig::default());
    let a = open(&router, &mut pool, 1, TabMode::Foreground);
    let entry = router.entry(a.entry_id).unwrap();
    let value = Rc::new(Cell::new(0));
    let applied = value.clone();

    entry
        .on_event(move |number: u32| {
            applied.set(number);

            Ok(())
        })
        .unwrap();
    let task = entry
        .activity("load", |run| async move { run.publish(42_u32) })
        .unwrap();

    pool.run_until_stalled();
    assert_eq!(value.get(), 42);
    let first = task.snapshot().unwrap().run_id;

    router.set_host_visibility(HostVisibility::Visible);
    pool.run_until_stalled();
    assert_eq!(task.snapshot().unwrap().state, TaskState::Completed);
    assert_eq!(task.snapshot().unwrap().run_id, first);
}

#[test]
fn cancellation_dropping_observers_and_failure_observation_are_explicit() {
    let (router, mut pool) = harness(RouterConfig::default());
    let done = Rc::new(Cell::new(false));
    let observed = done.clone();

    drop(
        router
            .app()
            .tasks()
            .spawn("eager", async move {
                observed.set(true);

                Ok(())
            })
            .unwrap(),
    );
    let failure = router
        .app()
        .tasks()
        .spawn("failure", async {
            Err(TaskError::Failed("fixture".into()))
        })
        .unwrap();

    let dropped = Rc::new(Cell::new(0));
    let guard = Dropped(dropped.clone());

    let cancelled = router
        .app()
        .tasks()
        .spawn("cancel", async move {
            let _guard = guard;

            futures::future::pending::<()>().await;

            Ok(())
        })
        .unwrap();

    assert!(cancelled.cancel().unwrap());
    assert!(!cancelled.cancel().unwrap());
    pool.run_until_stalled();
    assert!(done.get());
    assert_eq!(
        dropped.get(),
        1,
        "even an unpolled future releases captures"
    );
    assert_eq!(
        pool.run_until(cancelled.wait()).unwrap(),
        TaskState::Cancelled
    );
    assert_eq!(
        pool.run_until(failure.wait()).unwrap(),
        TaskState::Failed(TaskError::Failed("fixture".into()))
    );
    assert!(!failure.cancel().unwrap());
}

#[test]
fn publication_and_record_bounds_reject_overflow_and_report_expiration() {
    let (router, mut pool) = harness(RouterConfig {
        tasks: TaskConfig {
            capacity: 2,
            completed: 1,
            publications: 1,
        },
        ..RouterConfig::default()
    });

    let scope = router.app().tasks();

    scope.on_event(|_: u8| Ok(())).unwrap();
    let contexts = Rc::new(RefCell::new(Vec::new()));
    let observed = contexts.clone();

    let first = scope
        .spawn_with("first", move |run| async move {
            observed.borrow_mut().push(run);
            futures::future::pending::<()>().await;

            Ok(())
        })
        .unwrap();

    pool.run_until_stalled();
    let run = contexts.borrow()[0].clone();

    run.publish(1_u8).unwrap();
    assert_eq!(run.publish(2_u8), Err(TaskError::PublicationQueueFull));
    assert_eq!(run.publish(3_u32), Err(TaskError::UnregisteredEvent));
    let second = scope.spawn("second", async { Ok(()) }).unwrap();

    assert!(matches!(
        scope.spawn("overflow", async { Ok(()) }),
        Err(TaskError::Capacity)
    ));
    pool.run_until_stalled();
    assert_eq!(second.snapshot().unwrap().state, TaskState::Completed);
    first.cancel().unwrap();
    pool.run_until_stalled();
    assert_eq!(second.snapshot().unwrap_err(), TaskError::Expired);
}

#[test]
fn cancelling_the_host_driver_releases_work_and_rejects_new_admission() {
    let (router, mut pool) = harness(RouterConfig::default());
    let dropped = Rc::new(Cell::new(0));
    let guard = Dropped(dropped.clone());

    let task = router
        .app()
        .tasks()
        .spawn("owned", async move {
            let _guard = guard;

            futures::future::pending::<()>().await;

            Ok(())
        })
        .unwrap();

    pool.run_until_stalled();
    drop(pool);
    assert_eq!(dropped.get(), 1);
    assert_eq!(task.snapshot().unwrap().state, TaskState::Cancelled);
    assert!(matches!(
        router.app().tasks().spawn("late", async { Ok(()) }),
        Err(TaskError::OwnerDisposed)
    ));
}

#[test]
fn snapshots_keep_work_kinds_and_origins_after_the_source_tab_closes() {
    let (router, mut pool) = harness(RouterConfig::default());
    let a = open(&router, &mut pool, 1, TabMode::Live);

    let job = router
        .navigator()
        .tab(a.tab_id)
        .tasks()
        .spawn("download", async {
            futures::future::pending::<()>().await;

            Ok(())
        })
        .unwrap();

    pool.run_until_stalled();
    let snapshot = job.snapshot().unwrap();

    assert_eq!(snapshot.kind, TaskKind::Task);
    assert_eq!(snapshot.owner, TaskOwner::Tab(a.tab_id));
    assert_eq!(snapshot.origin.tab_id, Some(a.tab_id));
    assert_eq!(snapshot.origin.runtime_id, router.snapshot().runtime_id);
    commit(&mut pool, router.navigator().tab(a.tab_id).close());
    pool.run_until_stalled();
    assert_eq!(job.snapshot().unwrap().origin, snapshot.origin);
}

#[test]
fn pending_close_preserves_activity_until_disposal_actually_commits() {
    let (router, mut pool) = harness(RouterConfig::default());
    let a = open(&router, &mut pool, 1, TabMode::Live);
    let entry = router.entry(a.entry_id).unwrap();
    let runs = Rc::new(RefCell::new(Vec::new()));

    activity(&entry, false, runs.clone(), Rc::new(Cell::new(0)));
    entry.set_dirty(true).unwrap();
    let close = router.navigator().tab(a.tab_id).close();

    pool.run_until_stalled();
    let decision = router.snapshot().decisions[0].id;

    assert!(!runs.borrow()[0].cancellation().is_cancelled());
    router.navigator().respond(decision, "cancel").unwrap();
    pool.run_until(close.wait()).unwrap();
    assert!(!runs.borrow()[0].cancellation().is_cancelled());
    entry.set_dirty(false).unwrap();
    commit(&mut pool, router.navigator().tab(a.tab_id).close());
    assert!(runs.borrow()[0].cancellation().is_cancelled());
}

#[test]
fn rapid_eligibility_changes_coalesce_unstarted_runs_and_reject_every_old_context() {
    let (router, mut pool) = harness(RouterConfig::default());
    let a = open(&router, &mut pool, 1, TabMode::Foreground);
    let entry = router.entry(a.entry_id).unwrap();
    let runs = Rc::new(RefCell::new(Vec::new()));

    activity(&entry, false, runs.clone(), Rc::new(Cell::new(0)));
    pool.run_until_stalled();
    let first = runs.borrow()[0].clone();

    for _ in 0..1000 {
        router.set_host_visibility(HostVisibility::Hidden);
        router.set_host_visibility(HostVisibility::Visible);
    }

    pool.run_until_stalled();
    assert_eq!(
        runs.borrow().len(),
        2,
        "intermediate queued factories must not execute"
    );
    assert_eq!(first.publish(1_u32), Err(TaskError::StaleRun));
    assert!(!runs.borrow()[1].cancellation().is_cancelled());
}

#[test]
fn handler_failures_are_observable_and_stop_the_originating_run() {
    let (router, mut pool) = harness(RouterConfig::default());
    let scope = router.app().tasks();

    scope
        .on_event(|_: u64| Err(TaskError::Failed("handler fixture".into())))
        .unwrap();
    let contexts = Rc::new(RefCell::new(Vec::new()));
    let observed = contexts.clone();

    let task = scope
        .spawn_with("producer", move |run| async move {
            observed.borrow_mut().push(run.clone());
            run.publish(1_u64)?;
            futures::future::pending::<()>().await;

            Ok(())
        })
        .unwrap();

    pool.run_until_stalled();
    assert_eq!(
        task.snapshot().unwrap().state,
        TaskState::Failed(TaskError::Failed("handler fixture".into()))
    );
    assert!(contexts.borrow()[0].cancellation().is_cancelled());
}

#[test]
fn zero_terminal_retention_does_not_discard_already_accepted_results() {
    let (router, mut pool) = harness(RouterConfig {
        tasks: TaskConfig {
            completed: 0,
            ..TaskConfig::default()
        },
        ..RouterConfig::default()
    });

    let value = Rc::new(Cell::new(0));
    let observed = value.clone();
    let scope = router.app().tasks();

    scope
        .on_event(move |number: u32| {
            observed.set(number);

            Ok(())
        })
        .unwrap();
    let task = scope
        .spawn_with("result", |run| async move { run.publish(42_u32) })
        .unwrap();

    pool.run_until_stalled();
    assert_eq!(value.get(), 42);
    assert_eq!(task.snapshot().unwrap_err(), TaskError::Expired);
}

#[test]
fn dropping_an_unstarted_runtime_releases_queued_work_without_a_cycle() {
    let router = Router::<Destination>::new(RouterConfig::default());
    let drops = Rc::new(Cell::new(0));
    let guard = Dropped(drops.clone());

    let task = router
        .app()
        .tasks()
        .spawn("queued", async move {
            let _guard = guard;

            Ok(())
        })
        .unwrap();

    let mut waiting = Box::pin(task.wait());

    assert!(waiting.as_mut().now_or_never().is_none());
    drop(router);
    assert_eq!(drops.get(), 1);
    assert_eq!(task.snapshot().unwrap_err(), TaskError::OwnerDisposed);
    assert_eq!(
        futures::executor::block_on(waiting),
        Err(TaskError::OwnerDisposed)
    );
}
