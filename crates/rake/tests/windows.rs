//! Covers: window-scoped navigation, independent histories, guarded close and reload.
//! Real: public router, operation/decision scheduler, work lifetimes and checkpoint codec.
//! Doubles: an in-process physical-window adapter; GPUI windows are tested separately.
//! Requires: no display, network or external service.

use futures::{FutureExt, executor::LocalPool, future::LocalBoxFuture, task::LocalSpawnExt};
use rake::prelude::*;
use rake::reload::{ReloadMechanism, ReloadPhase, ReloadRequest};
use rake::router::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone, Route)]
enum Destination {
    #[route(id = "document", path = "/documents/{id}")]
    Document(u64),
}

#[derive(Clone, Default)]
struct Host {
    opened: Rc<RefCell<Vec<WindowId>>>,
    closed: Rc<RefCell<Vec<WindowId>>>,
    activated: Rc<RefCell<Vec<WindowId>>>,
    fail: Rc<Cell<bool>>,
    wait: Rc<Cell<bool>>,
}

impl WindowHost for Host {
    fn open(&self, id: WindowId, _: Activation) -> LocalBoxFuture<'static, Result<(), MountError>> {
        let host = self.clone();

        async move {
            host.opened.borrow_mut().push(id);

            if host.fail.get() {
                return Err(MountError("injected host failure".into()));
            }

            if host.wait.get() {
                futures::future::pending::<()>().await;
            }

            Ok(())
        }
        .boxed_local()
    }

    fn close(&self, id: WindowId) {
        self.closed.borrow_mut().push(id);
    }

    fn activate(&self, id: WindowId) {
        self.activated.borrow_mut().push(id);
    }
}

fn harness() -> (Router<Destination>, LocalPool, Host) {
    let router = Router::new(RouterConfig::default());
    let host = Host::default();

    router.window_host(host.clone()).unwrap();
    let pool = LocalPool::new();
    let driven = router.clone();

    pool.spawner()
        .spawn_local(async move {
            driven.run().await.unwrap();
        })
        .unwrap();

    (router, pool, host)
}

fn commit<T: Clone>(pool: &mut LocalPool, operation: Operation<T>) -> T {
    match pool.run_until(operation.wait()).unwrap() {
        Outcome::Committed(value) => value,
        _ => panic!("commit expected"),
    }
}

fn open(router: &Router<Destination>, pool: &mut LocalPool, target: OpenTarget, id: u64) -> Opened {
    commit(
        pool,
        router.navigator().open(Destination::Document(id), target),
    )
}

#[test]
fn windows_have_independent_selection_histories_and_bound_contextual_targets() {
    let (router, mut pool, host) = harness();
    let first = open(&router, &mut pool, OpenTarget::NewTab, 1);
    let second = open(&router, &mut pool, OpenTarget::NewWindow, 1);
    let bound = router.navigator().for_window(second.window_id);
    let submitted = bound.open(Destination::Document(2), OpenTarget::CurrentTab);

    let third = commit(
        &mut pool,
        bound.open(Destination::Document(3), OpenTarget::NewTab),
    );

    let entered = commit(&mut pool, submitted);

    assert_eq!(entered.tab_id, second.tab_id);
    assert_eq!(entered.window_id, second.window_id);
    assert_eq!(
        router.snapshot_in_window(first.window_id).unwrap().selected,
        Some(first.tab_id)
    );
    let snapshot = router.snapshot_in_window(second.window_id).unwrap();

    assert_eq!(snapshot.selected, Some(third.tab_id));
    assert_eq!(snapshot.tabs.len(), 2);
    assert_eq!(snapshot.tabs[0].entries.len(), 2);
    commit(&mut pool, bound.tab(second.tab_id).back());
    assert_eq!(
        router
            .snapshot_in_window(second.window_id)
            .unwrap()
            .selected,
        Some(third.tab_id)
    );
    let reused = commit(
        &mut pool,
        bound.reveal(Destination::Document(1), RevealScope::Window),
    );

    assert_eq!(reused.opened.tab_id, second.tab_id);
    let reused = commit(
        &mut pool,
        router
            .navigator()
            .reveal(Destination::Document(3), RevealScope::Application),
    );

    assert_eq!(reused.opened.window_id, second.window_id);
    assert_eq!(host.activated.borrow().last(), Some(&second.window_id));
    assert_eq!(
        router
            .snapshot_in_window(first.window_id)
            .unwrap()
            .tabs
            .len(),
        1
    );
    router.shutdown();
    pool.run_until_stalled();
}

#[test]
fn window_close_guards_hidden_history_blocks_only_its_host_and_revalidates_edits() {
    let (router, mut pool, _) = harness();
    let first = open(&router, &mut pool, OpenTarget::NewTab, 1);
    let second = open(&router, &mut pool, OpenTarget::NewWindow, 2);
    let dirty = router.entry(second.entry_id).unwrap();

    dirty.set_dirty(true).unwrap();
    commit(
        &mut pool,
        router
            .navigator()
            .tab(second.tab_id)
            .navigate(Destination::Document(3)),
    );
    let close = router.close_window(second.window_id);

    assert_eq!(close.id(), router.close_window(second.window_id).id());
    pool.run_until_stalled();
    let decision = router.snapshot().decisions[0].clone();

    assert_eq!(decision.window_id, second.window_id);
    assert_eq!(decision.entry_id, second.entry_id);
    assert!(matches!(
        pool.run_until(
            router
                .navigator()
                .for_window(second.window_id)
                .open(Destination::Document(4), OpenTarget::NewTab)
                .wait()
        ),
        Err(OperationError::WindowClosing)
    ));
    commit(
        &mut pool,
        router
            .navigator()
            .tab(first.tab_id)
            .navigate(Destination::Document(5)),
    );
    dirty.set_dirty(true).unwrap();
    assert!(router.navigator().respond(decision.id, "discard").is_err());
    pool.run_until_stalled();
    let decision = router.snapshot().decisions[0].clone();

    router.navigator().respond(decision.id, "cancel").unwrap();
    assert!(matches!(
        pool.run_until(close.wait()).unwrap(),
        Outcome::Cancelled(_)
    ));
    assert_eq!(router.snapshot().tabs.len(), 2);
    let close = router.close_window(second.window_id);

    pool.run_until_stalled();
    let decision = router.snapshot().decisions[0].clone();

    router.navigator().respond(decision.id, "discard").unwrap();
    commit(&mut pool, close);
    assert!(matches!(
        router.snapshot_in_window(second.window_id),
        Err(OperationError::WindowUnavailable)
    ));
    assert_eq!(router.snapshot().tabs.len(), 1);
    assert!(router.entry(second.entry_id).is_err());
    assert!(matches!(
        pool.run_until(
            router
                .navigator()
                .for_window(second.window_id)
                .open(Destination::Document(6), OpenTarget::NewTab)
                .wait()
        ),
        Err(OperationError::WindowUnavailable)
    ));
    router.shutdown();
    pool.run_until_stalled();
}

#[test]
fn failed_and_cancelled_window_creation_roll_back_provisional_hosts() {
    let (router, mut pool, host) = harness();

    open(&router, &mut pool, OpenTarget::NewTab, 1);
    host.fail.set(true);
    let failed = router
        .navigator()
        .open(Destination::Document(2), OpenTarget::NewWindow);

    assert!(matches!(
        pool.run_until(failed.wait()),
        Err(OperationError::Mount(_))
    ));
    assert_eq!(router.snapshot().windows.len(), 1);
    assert_eq!(router.snapshot().tabs.len(), 1);
    host.fail.set(false);
    host.wait.set(true);
    let pending = router
        .navigator()
        .open(Destination::Document(3), OpenTarget::NewWindow);

    pool.run_until_stalled();
    router.cancel(pending.id()).unwrap();
    assert!(matches!(
        pool.run_until(pending.wait()).unwrap(),
        Outcome::Cancelled(_)
    ));
    assert_eq!(router.snapshot().windows.len(), 1);
    assert_eq!(host.closed.borrow().len(), 2);
    router.shutdown();
    pool.run_until_stalled();
}

#[test]
fn new_window_access_decisions_are_presented_in_the_originating_host_before_creation() {
    let (router, mut pool, host) = harness();

    router
        .entry_guard(|context| async move {
            if matches!(context.destination, Destination::Document(2)) {
                GuardResult::Ask(DecisionRequest {
                    message: "Open this window?".into(),
                    choices: vec![
                        DecisionChoice::proceed("open", "Open"),
                        DecisionChoice::cancel("cancel", "Cancel"),
                    ],
                })
            } else {
                GuardResult::Allow
            }
        })
        .unwrap();
    let first = open(&router, &mut pool, OpenTarget::NewTab, 1);

    let pending = router
        .navigator()
        .open(Destination::Document(2), OpenTarget::NewWindow);

    pool.run_until_stalled();
    let decisions = router
        .snapshot_in_window(first.window_id)
        .unwrap()
        .decisions;

    assert_eq!(decisions.len(), 1);
    assert!(host.opened.borrow().is_empty());
    assert_eq!(host.activated.borrow().last(), Some(&first.window_id));
    router
        .navigator()
        .respond(decisions[0].id, "cancel")
        .unwrap();
    assert!(matches!(
        pool.run_until(pending.wait()).unwrap(),
        Outcome::Cancelled(_)
    ));
    assert!(host.opened.borrow().is_empty());
    let pending = router
        .navigator()
        .open(Destination::Document(2), OpenTarget::NewWindow);

    pool.run_until_stalled();
    let decision = router
        .snapshot_in_window(first.window_id)
        .unwrap()
        .decisions[0]
        .id;

    router.navigator().respond(decision, "open").unwrap();
    let second = commit(&mut pool, pending);

    assert_ne!(second.window_id, first.window_id);
    assert_eq!(&*host.opened.borrow(), &[second.window_id]);
    router.shutdown();
    pool.run_until_stalled();
}

#[test]
fn host_visibility_and_close_preserve_work_in_other_windows_and_application_scope() {
    let (router, mut pool, _) = harness();
    let first = open(&router, &mut pool, OpenTarget::NewTab, 1);
    let second = open(&router, &mut pool, OpenTarget::NewWindow, 2);

    let activity_a = router
        .entry(first.entry_id)
        .unwrap()
        .activity("a", |_| async { futures::future::pending().await })
        .unwrap();

    let activity_b = router
        .entry(second.entry_id)
        .unwrap()
        .activity("b", |_| async { futures::future::pending().await })
        .unwrap();

    let app = router
        .navigator()
        .for_window(first.window_id)
        .app()
        .tasks()
        .spawn("export", async { futures::future::pending().await })
        .unwrap();

    router
        .set_window_visibility(first.window_id, rake::tasks::HostVisibility::Visible)
        .unwrap();
    router
        .set_window_visibility(second.window_id, rake::tasks::HostVisibility::Visible)
        .unwrap();
    pool.run_until_stalled();
    assert_eq!(
        activity_b.snapshot().unwrap().origin.window_id,
        second.window_id
    );
    let run_b = activity_b.snapshot().unwrap().run_id;

    let revision = router.snapshot().revision;

    router
        .set_window_visibility(second.window_id, rake::tasks::HostVisibility::Visible)
        .unwrap();
    assert_eq!(
        router.snapshot().revision,
        revision,
        "unchanged visibility must not cause redraws"
    );

    router
        .set_window_visibility(first.window_id, rake::tasks::HostVisibility::Hidden)
        .unwrap();
    pool.run_until_stalled();
    assert!(!activity_a.snapshot().unwrap().eligible);
    assert!(activity_b.snapshot().unwrap().eligible);
    assert_eq!(activity_b.snapshot().unwrap().run_id, run_b);
    commit(&mut pool, router.close_window(first.window_id));
    assert!(activity_b.snapshot().unwrap().eligible);
    assert_eq!(
        app.snapshot().unwrap().state,
        rake::tasks::TaskState::Running
    );
    commit(
        &mut pool,
        router
            .navigator()
            .for_window(second.window_id)
            .open(Destination::Document(3), OpenTarget::NewTab),
    );
    router.shutdown();
    pool.run_until_stalled();
}

#[test]
fn checkpoint_restores_all_window_histories_and_selection_after_original_host_closes() {
    let (router, mut pool, _) = harness();
    let first = open(&router, &mut pool, OpenTarget::NewTab, 1);
    let second = open(&router, &mut pool, OpenTarget::NewWindow, 2);

    commit(
        &mut pool,
        router
            .navigator()
            .tab(second.tab_id)
            .navigate(Destination::Document(3)),
    );
    router
        .prepare_reload(ReloadRequest {
            id: "build".into(),
            mechanism: ReloadMechanism::NativeRestart,
        })
        .unwrap();
    pool.run_until_stalled();
    assert_eq!(router.reload_status().phase, ReloadPhase::Ready);
    let saved = router.reload_checkpoint("windows").unwrap();

    assert_eq!(saved.version, 2);
    assert_eq!(saved.windows.len(), 2);
    let replacement = Router::<Destination>::new(RouterConfig::default());

    replacement.window_host(Host::default()).unwrap();
    pool.run_until(replacement.restore_workspace(&saved, "windows"))
        .unwrap();

    for host in &saved.windows {
        let restored = replacement.snapshot_in_window(host.id).unwrap();

        assert_eq!(restored.selected, host.selected);
        assert_eq!(
            restored.tabs.len(),
            saved
                .tabs
                .iter()
                .filter(|tab| tab.window_id == host.id)
                .count()
        );
    }

    let mut invalid = saved.clone();

    invalid.windows[0].selected = Some(second.tab_id);
    let rejected = Router::<Destination>::new(RouterConfig::default());
    let host = Host::default();

    rejected.window_host(host.clone()).unwrap();
    assert!(
        pool.run_until(rejected.restore_workspace(&invalid, "windows"))
            .is_err()
    );
    assert!(rejected.snapshot().tabs.is_empty());
    assert!(
        host.opened.borrow().is_empty(),
        "validate every window before creating hosts"
    );
    router.finish_reload("build", false).unwrap();
    commit(&mut pool, router.close_window(first.window_id));
    router
        .prepare_reload(ReloadRequest {
            id: "next".into(),
            mechanism: ReloadMechanism::NativeRestart,
        })
        .unwrap();
    pool.run_until_stalled();
    let saved = router.reload_checkpoint("windows").unwrap();
    let replacement = Router::<Destination>::new(RouterConfig::default());

    replacement.window_host(Host::default()).unwrap();
    pool.run_until(replacement.restore_workspace(&saved, "windows"))
        .unwrap();
    let snapshot = replacement.snapshot();

    assert_eq!(snapshot.windows.len(), 1);
    assert_eq!(snapshot.window_id, second.window_id);
    assert_eq!(snapshot.selected, Some(second.tab_id));
    assert_eq!(snapshot.tabs[0].entries.len(), 2);
    router.shutdown();
    pool.run_until_stalled();
}

#[test]
fn closing_last_window_keeps_runtime_and_application_work_alive_and_can_reopen() {
    let (router, mut pool, _) = harness();
    let first = open(&router, &mut pool, OpenTarget::NewTab, 1);
    let runtime = router.runtime_id();

    let app = router
        .navigator()
        .app()
        .tasks()
        .spawn("export", async { futures::future::pending().await })
        .unwrap();

    pool.run_until_stalled();
    commit(&mut pool, router.close_window(first.window_id));
    assert!(router.snapshot().windows.is_empty());
    assert!(router.snapshot().tabs.is_empty());
    assert_eq!(
        app.snapshot().unwrap().state,
        rake::tasks::TaskState::Running
    );
    router
        .prepare_reload(ReloadRequest {
            id: "empty".into(),
            mechanism: ReloadMechanism::NativeRestart,
        })
        .unwrap();
    pool.run_until_stalled();
    let empty = router.reload_checkpoint("windows").unwrap();
    let replacement = Router::<Destination>::new(RouterConfig::default());
    let host = Host::default();

    replacement.window_host(host.clone()).unwrap();
    pool.run_until(replacement.restore_workspace(&empty, "windows"))
        .unwrap();
    assert!(replacement.snapshot().windows.is_empty());
    assert_eq!(
        host.closed.borrow().len(),
        1,
        "retire the unused initial host"
    );
    router.finish_reload("empty", false).unwrap();
    let reopened = open(&router, &mut pool, OpenTarget::NewWindow, 2);

    assert_ne!(reopened.window_id, first.window_id);
    assert_eq!(router.runtime_id(), runtime);
    assert_eq!(
        router
            .snapshot_in_window(reopened.window_id)
            .unwrap()
            .selected,
        Some(reopened.tab_id)
    );
    router.shutdown();
    pool.run_until_stalled();
}

#[test]
fn legacy_single_window_checkpoint_remains_compatible() {
    let (router, mut pool, _) = harness();
    let first = open(&router, &mut pool, OpenTarget::NewTab, 1);

    router
        .prepare_reload(ReloadRequest {
            id: "legacy".into(),
            mechanism: ReloadMechanism::NativeRestart,
        })
        .unwrap();
    pool.run_until_stalled();
    let saved = router.reload_checkpoint("windows").unwrap();
    let mut legacy = serde_json::to_value(saved).unwrap();

    legacy["version"] = 1.into();
    legacy.as_object_mut().unwrap().remove("windows");

    for tab in legacy["tabs"].as_array_mut().unwrap() {
        tab.as_object_mut().unwrap().remove("window_id");
    }

    let legacy = serde_json::from_value(legacy).unwrap();
    let replacement = Router::<Destination>::new(RouterConfig::default());

    pool.run_until(replacement.restore_workspace(&legacy, "windows"))
        .unwrap();
    assert_eq!(replacement.snapshot().selected, Some(first.tab_id));
    assert_eq!(replacement.snapshot().tabs[0].window_id, first.window_id);
    router.shutdown();
    pool.run_until_stalled();
}

#[test]
fn shared_control_scopes_queries_and_retry_safe_close_to_the_requested_host() {
    use rake::control::{Command, Message, PROTOCOL_VERSION, Query, Reply, Request};
    let (router, mut pool, _) = harness();
    let first = open(&router, &mut pool, OpenTarget::NewTab, 1);
    let second = open(&router, &mut pool, OpenTarget::NewWindow, 2);
    let client = router.client();

    let query = client
        .request(Request {
            protocol: PROTOCOL_VERSION,
            runtime_id: router.runtime_id(),
            key: None,
            message: Message::Query(Query::Window {
                window: second.window_id,
            }),
        })
        .unwrap();

    let Reply::Snapshot(snapshot) = query else {
        panic!("window snapshot")
    };

    assert_eq!(snapshot.selected, Some(second.tab_id));
    assert_eq!(snapshot.tabs.len(), 1);
    assert_eq!(snapshot.tabs[0].window_id, second.window_id);
    let request = Request {
        protocol: PROTOCOL_VERSION,
        runtime_id: router.runtime_id(),
        key: Some("close-second".into()),
        message: Message::Command(Command::CloseWindow {
            window: second.window_id,
        }),
    };

    let reply = client.request(request.clone()).unwrap();

    assert_eq!(
        serde_json::to_value(&reply).unwrap(),
        serde_json::to_value(client.request(request.clone()).unwrap()).unwrap()
    );
    pool.run_until_stalled();
    assert_eq!(router.snapshot().tabs[0].id, first.tab_id);
    assert_eq!(router.snapshot().windows.len(), 1);
    assert_eq!(
        serde_json::to_value(&reply).unwrap(),
        serde_json::to_value(client.request(request).unwrap()).unwrap()
    );
    router.shutdown();
    pool.run_until_stalled();
}
