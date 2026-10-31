//! Covers: whole-workspace quit, save/discard/cancel, finite jobs and cancellation boundaries.
//! Real: public router, task driver, typed commands, guard decisions and JSON control dispatch.
//! Doubles: headless view mounts, explicit completion gates. Requires: no display or network.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use futures::FutureExt;
use futures::channel::oneshot;
use futures::executor::LocalPool;
use futures::task::LocalSpawnExt;
use rake::control::{Command, Message, PROTOCOL_VERSION, Query, Reply, Request};
use rake::prelude::*;
use rake::quit::{QuitJobs, QuitOptions, QuitPhase};
use rake::reload::{ReloadError, ReloadMechanism, ReloadRequest};

use rake::router::{
    CancelReason, DecisionError, DisposeReason, EntryHandle, GuardKind, GuardReason, NoRoute,
    Operation, OperationError, OperationState,
};

use rake::tasks::TaskState;

#[derive(Clone, Route)]
enum Destination {
    #[route(id = "page", path = "/page/{id}")]
    Page(u64),
}

fn harness() -> (Router<Destination>, LocalPool) {
    let router = Router::new(RouterConfig::default());
    let pool = running(&router);

    (router, pool)
}

fn running<R: Route>(router: &Router<R>) -> LocalPool {
    let pool = LocalPool::new();
    let driven = router.clone();

    pool.spawner()
        .spawn_local(async move {
            driven.run().await.unwrap();
        })
        .unwrap();

    pool
}

fn commit<T: Clone>(pool: &mut LocalPool, operation: Operation<T>) -> T {
    match pool.run_until(operation.wait()).unwrap() {
        Outcome::Committed(value) => value,
        _ => panic!("expected commit"),
    }
}

fn open(router: &Router<Destination>, pool: &mut LocalPool, id: u64) -> EntryHandle<Destination> {
    let opened = commit(
        pool,
        router
            .navigator()
            .open(Destination::Page(id), OpenTarget::NewTab),
    );

    router.entry(opened.entry_id).unwrap()
}

#[test]
fn app_quit_guard_denies_a_route_free_quit() {
    let router = Router::<NoRoute>::new(RouterConfig::default());
    let invoked = Rc::new(Cell::new(false));
    let observed = invoked.clone();

    router
        .quit_guard(move |_| {
            observed.set(true);
            async { Err(GuardReason("application is busy".into())) }
        })
        .unwrap();
    let mut pool = running(&router);
    let quit = router.quit(QuitOptions::default());

    assert!(matches!(
        pool.run_until(quit.wait()).unwrap(),
        Outcome::Blocked(_)
    ));
    assert!(invoked.get());
    assert_eq!(router.quit_status().phase, QuitPhase::Blocked);
}

#[test]
fn app_quit_guard_cancellation_wins_while_the_guard_is_pending() {
    let router = Router::<NoRoute>::new(RouterConfig::default());
    let (sender, receiver) = oneshot::channel::<()>();
    let receiver = Rc::new(RefCell::new(Some(receiver)));

    router
        .quit_guard(move |_| {
            let receiver = receiver.borrow_mut().take().unwrap();

            async move {
                receiver.await.unwrap();
                Ok(())
            }
        })
        .unwrap();
    let mut pool = running(&router);
    let quit = router.quit(QuitOptions::default());

    pool.run_until_stalled();
    assert_eq!(router.quit_status().phase, QuitPhase::Guarding);
    assert!(router.cancel(quit.id()).unwrap());
    assert!(matches!(
        pool.run_until(quit.wait()).unwrap(),
        Outcome::Cancelled(CancelReason::Requested)
    ));
    assert_eq!(router.quit_status().phase, QuitPhase::Cancelled);
    drop(sender);
}

#[test]
fn app_quit_guard_approves_with_operation_context_and_options() {
    let router = Router::<NoRoute>::new(RouterConfig::default());
    let operation_id = Rc::new(Cell::new(None));
    let observed_id = operation_id.clone();
    let options = Rc::new(Cell::new(None));
    let observed_options = options.clone();

    router
        .quit_guard(move |context| {
            observed_id.set(Some(context.operation_id));
            observed_options.set(Some(context.options));
            async { Ok(()) }
        })
        .unwrap();
    let mut pool = running(&router);
    let expected_options = QuitOptions {
        jobs: QuitJobs::Cancel,
    };
    let quit = router.quit(expected_options);
    let expected_id = quit.id();

    commit(&mut pool, quit);
    assert_eq!(operation_id.get(), Some(expected_id));
    assert_eq!(options.get(), Some(expected_options));
    assert_eq!(router.quit_status().phase, QuitPhase::Ready);
}

#[test]
fn app_quit_guard_runs_after_finite_work_settles() {
    let router = Router::<NoRoute>::new(RouterConfig::default());
    let order = Rc::new(RefCell::new(Vec::new()));
    let task_order = order.clone();
    let (sender, receiver) = oneshot::channel::<()>();

    router
        .app()
        .tasks()
        .spawn("flush", async move {
            receiver.await.unwrap();
            task_order.borrow_mut().push("task");
            Ok(())
        })
        .unwrap();

    let guard_order = order.clone();
    router
        .quit_guard(move |_| {
            guard_order.borrow_mut().push("guard");
            async { Ok(()) }
        })
        .unwrap();
    let mut pool = running(&router);
    let quit = router.quit(QuitOptions::default());

    pool.run_until_stalled();
    assert_eq!(router.quit_status().phase, QuitPhase::WaitingForJobs);
    assert!(order.borrow().is_empty());

    sender.send(()).unwrap();
    commit(&mut pool, quit);
    assert_eq!(&*order.borrow(), &["task", "guard"]);
}

fn respond(router: &Router<Destination>, choice: &str) {
    let decisions = router.snapshot().decisions;

    assert_eq!(decisions.len(), 1);
    router.navigator().respond(decisions[0].id, choice).unwrap();
}

fn wire(router: &Router<Destination>, key: Option<&str>, message: Message) -> Reply {
    let request = Request {
        protocol: PROTOCOL_VERSION,
        runtime_id: router.runtime_id(),
        key: key.map(String::from),
        message,
    };

    let bytes = serde_json::to_vec(&request).unwrap();

    router
        .client()
        .request(serde_json::from_slice(&bytes).unwrap())
        .unwrap()
}

struct Dropped(Rc<Cell<usize>>);

impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

#[test]
fn guards_hidden_tabs_and_both_history_directions_without_partial_disposal() {
    let (router, mut pool) = harness();
    let first = open(&router, &mut pool, 1);
    let tab = router.navigator().tab(first.tab_id().unwrap());
    let second = commit(&mut pool, tab.navigate(Destination::Page(2)));
    let third = commit(&mut pool, tab.navigate(Destination::Page(3)));

    commit(&mut pool, tab.back());
    let hidden = open(&router, &mut pool, 4);

    let entries = [
        first,
        router.entry(second.entry_id).unwrap(),
        router.entry(third.entry_id).unwrap(),
        hidden,
    ];

    for entry in &entries {
        entry.set_dirty(true).unwrap();
    }

    let quit = router.quit(QuitOptions::default());

    pool.run_until_stalled();
    respond(&router, "discard");
    pool.run_until_stalled();
    respond(&router, "cancel");
    assert!(matches!(
        pool.run_until(quit.wait()).unwrap(),
        Outcome::Cancelled(CancelReason::Decision)
    ));
    assert_eq!(router.quit_status().phase, QuitPhase::Cancelled);
    assert!(entries.iter().all(|entry| entry.id().is_ok()));
    assert_eq!(router.snapshot().tabs.len(), 2);

    let quit = router.navigator().quit(QuitOptions::default());

    for entry in &entries {
        pool.run_until_stalled();
        assert_eq!(router.snapshot().decisions[0].entry_id, entry.id().unwrap());
        respond(&router, "discard");
    }

    commit(&mut pool, quit.clone());
    assert_eq!(router.quit_status().phase, QuitPhase::Ready);
    assert!(router.snapshot().tabs.is_empty());
    assert!(
        entries
            .iter()
            .all(|entry| entry.id() == Err(OperationError::ClosedTarget))
    );
    assert!(!router.cancel(quit.id()).unwrap());
    assert!(matches!(
        router.app().tasks().spawn("late", async { Ok(()) }),
        Err(TaskError::OwnerDisposed)
    ));
    assert_eq!(
        pool.run_until(
            router
                .navigator()
                .open(Destination::Page(5), OpenTarget::NewTab)
                .wait()
        ),
        Err(OperationError::Quitting)
    );
}

#[test]
fn wait_keeps_finite_jobs_and_activities_until_approval_then_drops_all_futures() {
    let (router, mut pool) = harness();
    let entry = open(&router, &mut pool, 1);
    let activity_drops = Rc::new(Cell::new(0));
    let drops = activity_drops.clone();

    entry
        .activity("live", move |_| {
            let guard = Dropped(drops.clone());

            async move {
                let _guard = guard;

                futures::future::pending::<()>().await;

                Ok(())
            }
        })
        .unwrap();
    let (sender, receiver) = oneshot::channel::<()>();

    let task = router
        .app()
        .tasks()
        .spawn("export", async move {
            receiver.await.unwrap();

            Ok(())
        })
        .unwrap();

    pool.run_until_stalled();
    let quit = router.quit(QuitOptions::default());

    pool.run_until_stalled();
    assert_eq!(router.quit_status().phase, QuitPhase::WaitingForJobs);
    assert_eq!(router.quit_status().pending_tasks, vec![task.id()]);
    assert_eq!(activity_drops.get(), 0);
    assert_eq!(task.snapshot().unwrap().state, TaskState::Running);
    sender.send(()).unwrap();
    commit(&mut pool, quit);
    assert_eq!(task.snapshot().unwrap().state, TaskState::Completed);
    assert_eq!(activity_drops.get(), 1);
    assert_eq!(router.quit_status().phase, QuitPhase::Ready);
}

#[test]
fn cancel_policy_waits_for_dirty_entry_approval_before_cancelling_finite_tasks() {
    let (router, mut pool) = harness();
    let entry = open(&router, &mut pool, 1);

    entry.set_dirty(true).unwrap();
    let drops = Rc::new(Cell::new(0));
    let guard = Dropped(drops.clone());

    let task = router
        .app()
        .tasks()
        .spawn("download", async move {
            let _guard = guard;

            futures::future::pending::<()>().await;

            Ok(())
        })
        .unwrap();

    pool.run_until_stalled();
    let options = QuitOptions {
        jobs: QuitJobs::Cancel,
    };

    let quit = router.quit(options);

    pool.run_until_stalled();
    assert_eq!(task.snapshot().unwrap().state, TaskState::Running);
    respond(&router, "cancel");
    pool.run_until(quit.wait()).unwrap();
    assert_eq!(drops.get(), 0);
    let quit = router.quit(options);

    pool.run_until_stalled();
    respond(&router, "discard");
    commit(&mut pool, quit);
    assert_eq!(task.snapshot().unwrap().state, TaskState::Cancelled);
    assert_eq!(drops.get(), 1);
}

#[test]
fn cancelled_quit_preserves_entries_and_reopens_admission() {
    let (router, mut pool) = harness();
    let entry = open(&router, &mut pool, 1);

    entry.set_dirty(true).unwrap();
    let quit = router.quit(QuitOptions::default());

    pool.run_until_stalled();
    let decision = router.snapshot().decisions[0].id;

    assert!(router.cancel(quit.id()).unwrap());
    pool.run_until(quit.wait()).unwrap();
    assert!(router.snapshot().decisions.is_empty());
    assert_eq!(
        router.navigator().respond(decision, "discard"),
        Err(DecisionError::Obsolete)
    );
    assert!(entry.is_dirty().unwrap());
    let quit = router.quit(QuitOptions::default());

    pool.run_until_stalled();
    respond(&router, "cancel");
    entry.set_dirty(true).unwrap();
    assert!(matches!(
        pool.run_until(quit.wait()).unwrap(),
        Outcome::Cancelled(CancelReason::Decision)
    ));
    open(&router, &mut pool, 2);
}

#[test]
fn late_job_edits_and_stale_responses_require_fresh_approval() {
    let (router, mut pool) = harness();
    let entry = open(&router, &mut pool, 1);

    entry.set_dirty(true).unwrap();
    let changed = entry.clone();
    let (sender, receiver) = oneshot::channel::<()>();

    router
        .app()
        .tasks()
        .spawn("edit", async move {
            receiver.await.unwrap();
            changed.set_dirty(true).unwrap();

            Ok(())
        })
        .unwrap();
    let quit = router.quit(QuitOptions::default());

    pool.run_until_stalled();
    let stale = router.snapshot().decisions[0].id;

    respond(&router, "discard");
    pool.run_until_stalled();
    assert_eq!(router.quit_status().phase, QuitPhase::WaitingForJobs);
    sender.send(()).unwrap();
    pool.run_until_stalled();
    assert_ne!(router.snapshot().decisions[0].id, stale);
    assert_eq!(
        router.navigator().respond(stale, "discard"),
        Err(DecisionError::Obsolete)
    );
    respond(&router, "discard");
    commit(&mut pool, quit);
}

#[rake::data(schema)]
#[derive(rake::Command)]
#[command(id = "test.save", capability = Headless)]
struct Save;

#[test]
fn save_preparation_can_await_a_shared_typed_command_without_holding_router_lanes() {
    let (router, mut pool) = harness();
    let entry = open(&router, &mut pool, 1);

    entry.set_dirty(true).unwrap();
    let (sender, receiver) = oneshot::channel::<()>();
    let sender = Rc::new(RefCell::new(Some(sender)));
    let saved = entry.clone();

    router
        .client()
        .commands()
        .register::<Save, _, _>(move |_, _| {
            let saved = saved.clone();
            let sender = sender.borrow_mut().take().unwrap();

            async move {
                saved.set_dirty(false).unwrap();
                sender.send(()).unwrap();

                Ok(())
            }
        })
        .unwrap();
    let navigator = router.navigator();
    let receiver = Rc::new(RefCell::new(Some(receiver)));

    entry
        .on_dispose(move |context| {
            assert!(matches!(
                context.kind,
                GuardKind::Dispose(DisposeReason::QuitApplication)
            ));
            let navigator = navigator.clone();
            let receiver = receiver.clone();

            async move {
                if !context.dirty {
                    return GuardResult::Allow;
                }

                GuardResult::Ask(DecisionRequest {
                    message: "Save before quitting?".into(),
                    choices: vec![
                        DecisionChoice::proceed("save", "Save").prepare(move || {
                            let reply = navigator.dispatch(Save, "save-on-quit".into()).unwrap();
                            assert!(matches!(reply, Reply::Submitted { .. }));
                            let receiver = receiver.borrow_mut().take().unwrap();
                            async move {
                                receiver.await.unwrap();
                                Ok(())
                            }
                        }),
                        DecisionChoice::proceed("discard", "Discard"),
                        DecisionChoice::cancel("cancel", "Cancel"),
                    ],
                })
            }
        })
        .unwrap();
    let quit = router.quit(QuitOptions::default());

    pool.run_until_stalled();
    respond(&router, "save");
    commit(&mut pool, quit);
    assert_eq!(router.quit_status().phase, QuitPhase::Ready);
}

#[test]
fn failed_save_blocks_quit_and_a_choice_invalidated_after_response_does_not_save() {
    let (router, mut pool) = harness();
    let entry = open(&router, &mut pool, 1);
    let saves = Rc::new(Cell::new(0));
    let saved = saves.clone();

    entry
        .on_dispose(move |_| {
            let saved = saved.clone();

            async move {
                GuardResult::Ask(DecisionRequest {
                    message: "Save?".into(),
                    choices: vec![DecisionChoice::proceed("save", "Save").prepare(move || {
                        saved.set(saved.get() + 1);
                        async { Err(GuardReason("disk full".into())) }
                    })],
                })
            }
        })
        .unwrap();
    let quit = router.quit(QuitOptions::default());

    pool.run_until_stalled();
    respond(&router, "save");
    entry.set_dirty(true).unwrap();
    pool.run_until_stalled();
    assert_eq!(saves.get(), 0);
    respond(&router, "save");
    assert!(matches!(
        pool.run_until(quit.wait()).unwrap(),
        Outcome::Blocked(_)
    ));
    assert_eq!(saves.get(), 1);
    assert_eq!(router.quit_status().phase, QuitPhase::Blocked);
    assert!(entry.id().is_ok());
    open(&router, &mut pool, 2);
}

#[test]
fn shared_control_requests_join_quit_and_preserve_first_policy_and_retry_identity() {
    let (router, mut pool) = harness();
    let entry = open(&router, &mut pool, 1);

    entry.set_dirty(true).unwrap();
    let command = Message::Command(Command::Quit {
        options: QuitOptions::default(),
    });

    let Reply::Submitted { operation_id } = wire(&router, Some("quit"), command.clone()) else {
        panic!()
    };

    assert_eq!(
        router
            .navigator()
            .quit(QuitOptions {
                jobs: QuitJobs::Cancel
            })
            .id(),
        operation_id
    );

    let Reply::Submitted {
        operation_id: retry,
    } = wire(&router, Some("quit"), command)
    else {
        panic!()
    };

    assert_eq!(operation_id, retry);
    assert_eq!(router.quit_status().options.jobs, QuitJobs::Wait);
    pool.run_until_stalled();
    assert!(
        matches!(wire(&router, None, Message::Query(Query::Quit {})), Reply::Quit(status) if status.phase == QuitPhase::Guarding)
    );
    assert!(matches!(
        wire(&router, None, Message::Query(Query::Decisions {})),
        Reply::Data(_)
    ));
    respond(&router, "discard");
    pool.run_until_stalled();
    assert_eq!(
        router.operation(operation_id).unwrap(),
        OperationState::Committed
    );
    assert!(
        matches!(wire(&router, None, Message::Query(Query::Quit {})), Reply::Quit(status) if status.phase == QuitPhase::Ready)
    );
}

#[test]
fn cancellation_wins_simultaneous_job_readiness_and_synchronous_guard_cancellation() {
    let (router, mut pool) = harness();
    let entry = open(&router, &mut pool, 1);
    let (sender, receiver) = oneshot::channel::<()>();

    router
        .app()
        .tasks()
        .spawn("job", async move {
            receiver.await.unwrap();

            Ok(())
        })
        .unwrap();
    let quit = router.quit(QuitOptions::default());

    pool.run_until_stalled();
    sender.send(()).unwrap();
    router.cancel(quit.id()).unwrap();
    assert!(matches!(
        pool.run_until(quit.wait()).unwrap(),
        Outcome::Cancelled(_)
    ));
    assert!(entry.id().is_ok());

    let client = Rc::new(router.client());
    let weak = Rc::downgrade(&client);
    let deny = Rc::new(Cell::new(false));
    let guard_deny = deny.clone();

    entry
        .on_dispose(move |context| {
            let client = weak.upgrade().unwrap();

            client
                .request(Request {
                    protocol: PROTOCOL_VERSION,
                    runtime_id: client.info().runtime_id,
                    key: Some(format!("cancel-inside-guard-{}", context.operation_id)),
                    message: Message::Command(Command::CancelOperation {
                        operation: context.operation_id,
                    }),
                })
                .unwrap();
            let deny = guard_deny.get();

            async move {
                if deny {
                    GuardResult::Deny(GuardReason("denied".into()))
                } else {
                    GuardResult::Allow
                }
            }
        })
        .unwrap();

    for denied in [false, true] {
        deny.set(denied);
        let quit = router.quit(QuitOptions::default());

        assert!(matches!(
            pool.run_until(quit.wait()).unwrap(),
            Outcome::Cancelled(_)
        ));
        assert!(entry.id().is_ok());
    }
}

#[test]
fn reload_and_quit_are_exclusive_and_dropping_an_unstarted_runtime_has_no_cycle() {
    let (router, mut pool) = harness();
    let entry = open(&router, &mut pool, 1);

    entry.set_dirty(true).unwrap();
    let quit = router.quit(QuitOptions::default());

    let request = ReloadRequest {
        id: "build".into(),
        mechanism: ReloadMechanism::NativeCodeReplacement,
    };

    assert_eq!(
        router.prepare_reload(request.clone()).unwrap_err(),
        ReloadError::Busy
    );
    router.cancel(quit.id()).unwrap();
    pool.run_until(quit.wait()).unwrap();
    router.prepare_reload(request).unwrap();
    assert_eq!(
        pool.run_until(router.quit(QuitOptions::default()).wait()),
        Err(OperationError::Reloading)
    );
    router.shutdown();

    let router = Router::<Destination>::new(RouterConfig::default());
    let quit = router.quit(QuitOptions::default());

    drop(router);
    assert_eq!(
        futures::executor::block_on(quit.wait()),
        Err(OperationError::RuntimeShutdown)
    );
}

#[test]
fn existing_operations_settle_or_cancel_before_quit_scans_the_final_workspace() {
    for policy in [QuitJobs::Wait, QuitJobs::Cancel] {
        let router = Router::<Destination>::new(RouterConfig::default());
        let (sender, receiver) = oneshot::channel::<()>();
        let receiver = Rc::new(RefCell::new(Some(receiver)));

        router
            .entry_guard(move |_| {
                let receiver = receiver.borrow_mut().take().unwrap();

                async move {
                    receiver.await.unwrap();

                    GuardResult::Allow
                }
            })
            .unwrap();
        let opened = router
            .navigator()
            .open(Destination::Page(1), OpenTarget::NewTab);

        let mut pool = LocalPool::new();
        let driven = router.clone();

        pool.spawner()
            .spawn_local(async move {
                driven.run().await.unwrap();
            })
            .unwrap();
        pool.run_until_stalled();
        let quit = router.quit(QuitOptions { jobs: policy });

        pool.run_until_stalled();

        if policy == QuitJobs::Wait {
            sender.send(()).unwrap();
        }

        commit(&mut pool, quit);
        let outcome = pool.run_until(opened.wait()).unwrap();

        assert_eq!(
            matches!(outcome, Outcome::Committed(_)),
            policy == QuitJobs::Wait
        );
        assert!(router.snapshot().tabs.is_empty());
    }
}

#[test]
fn dropping_a_pending_observer_does_not_cancel_quit_but_driver_teardown_does() {
    let (router, mut pool) = harness();
    let entry = open(&router, &mut pool, 1);

    entry.set_dirty(true).unwrap();
    let quit = router.quit(QuitOptions::default());
    let id = quit.id();

    drop(quit);
    pool.run_until_stalled();
    assert!(matches!(
        router.operation(id).unwrap(),
        OperationState::AwaitingDecision(_)
    ));
    let wait = router.quit(QuitOptions::default()).wait().boxed_local();

    drop(pool);
    assert_eq!(
        futures::executor::block_on(wait),
        Err(OperationError::RuntimeShutdown)
    );
}

#[cfg(feature = "updates")]
#[test]
fn update_handoff_waits_for_committed_quit_and_can_fail_without_forcing_exit() {
    let (router, mut pool) = harness();
    let invoked = Rc::new(Cell::new(false));
    let observed = Rc::clone(&invoked);
    let handoff = rake::updates::RestartHandoff::new(move || {
        observed.set(true);
        Ok(())
    });
    assert!(router.complete_update_restart().is_err());
    let quit = router
        .quit_for_update(handoff, QuitOptions::default())
        .unwrap();
    assert!(!invoked.get());
    commit(&mut pool, quit);
    assert!(!invoked.get());
    assert_eq!(
        router.complete_update_restart().unwrap(),
        rake::updates::RestartDisposition::ExitHost
    );
    assert!(invoked.get());
    assert_eq!(
        router.complete_update_restart().unwrap(),
        rake::updates::RestartDisposition::ExitHost
    );
    let (router, mut pool) = harness();
    let handoff = rake::updates::RestartHandoff::new(|| {
        Err(rake::updates::Error::Provider(
            "native installer unavailable".into(),
        ))
    });
    let quit = router
        .quit_for_update(handoff, QuitOptions::default())
        .unwrap();
    commit(&mut pool, quit);
    assert!(router.complete_update_restart().is_err());
    assert!(
        router
            .update_restart_error()
            .unwrap()
            .contains("native installer unavailable")
    );
}

#[cfg(feature = "updates")]
#[test]
fn dirty_document_cancellation_discards_the_update_handoff() {
    let (router, mut pool) = harness();
    let entry = open(&router, &mut pool, 1);
    entry.set_dirty(true).unwrap();
    let installed = Rc::new(Cell::new(false));
    let invoked = Rc::clone(&installed);
    let quit = router
        .quit_for_update(
            rake::updates::RestartHandoff::new(move || {
                invoked.set(true);
                Ok(())
            }),
            QuitOptions::default(),
        )
        .unwrap();
    pool.run_until_stalled();
    respond(&router, "cancel");
    assert!(matches!(
        pool.run_until(quit.wait()).unwrap(),
        Outcome::Cancelled(_)
    ));
    assert!(!installed.get());
    assert!(entry.id().is_ok());
    assert!(router.complete_update_restart().is_err());
    // A later ordinary quit cannot accidentally revive the declined installer.
    let quit = router.quit(QuitOptions::default());
    pool.run_until_stalled();
    respond(&router, "discard");
    commit(&mut pool, quit);
    assert_eq!(
        router.complete_update_restart().unwrap(),
        rake::updates::RestartDisposition::ExitHost
    );
    assert!(!installed.get());
}
