//! Covers: retained resource identity and mount failure rollback.
//! Real: scheduler, transactional commit, resource owner and entry lifetime signals.
//! Doubles: small local resources replacing actual GPUI entities.
//! Requires: no GPU or external resources.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use futures::executor::LocalPool;
use futures::task::LocalSpawnExt;

use super::{Mount, Resource};
use crate::diagnostics::Collector;
use crate::router::{
    Admission, BrowserCheckpoint, HistoryChange, MountError, NavigationState, OpenTarget,
    OperationError, Outcome, Route, Router, RouterConfig,
};

#[derive(Clone, crate::Route)]
enum Destination {
    #[route(id = "home", path = "/")]
    Home,
    #[route(id = "other", path = "/other")]
    Other,
    #[route(id = "broken", path = "/broken")]
    Broken,
    #[route(id = "item", path = "/items/{id}")]
    Item(String),
}

struct WindowHost(Rc<Cell<usize>>);

struct CountingHost {
    opened: Rc<Cell<usize>>,
    closed: Rc<Cell<usize>>,
}

impl crate::router::WindowHost for CountingHost {
    fn open(
        &self,
        _: crate::router::WindowId,
        _: crate::router::Activation,
    ) -> futures::future::LocalBoxFuture<'static, Result<(), MountError>> {
        self.opened.set(self.opened.get() + 1);
        Box::pin(async { Ok(()) })
    }

    fn close(&self, _: crate::router::WindowId) {
        self.closed.set(self.closed.get() + 1);
    }

    fn activate(&self, _: crate::router::WindowId) {}
}

impl crate::router::WindowHost for WindowHost {
    fn open(
        &self,
        _: crate::router::WindowId,
        _: crate::router::Activation,
    ) -> futures::future::LocalBoxFuture<'static, Result<(), MountError>> {
        Box::pin(async { Ok(()) })
    }

    fn close(&self, _: crate::router::WindowId) {
        self.0.set(self.0.get() + 1);
    }

    fn activate(&self, _: crate::router::WindowId) {}
}

#[test]
fn failed_route_mount_closes_the_new_physical_host_without_changing_existing_windows() {
    let mount: Mount<Destination> = Rc::new(|destination, _, _| {
        Box::pin(async move {
            if matches!(destination, Destination::Broken) {
                Err(MountError("injected route mount failure".into()))
            } else {
                Ok(Rc::new(()) as Resource)
            }
        })
    });

    let router = Router::with_mount(RouterConfig::default(), Collector::default(), mount);
    let closes = Rc::new(Cell::new(0));

    router.window_host(WindowHost(closes.clone())).unwrap();
    let mut pool = LocalPool::new();
    let driven = router.clone();

    pool.spawner()
        .spawn_local(async move {
            driven.run().await.unwrap();
        })
        .unwrap();
    pool.run_until(
        router
            .navigator()
            .open(Destination::Home, OpenTarget::NewTab)
            .wait(),
    )
    .unwrap();
    let before = router.snapshot();

    assert!(matches!(
        pool.run_until(
            router
                .navigator()
                .open(Destination::Broken, OpenTarget::NewWindow)
                .wait()
        ),
        Err(OperationError::Mount(_))
    ));
    assert_eq!(closes.get(), 1);
    let after = router.snapshot();

    assert_eq!(before.windows.len(), after.windows.len());
    assert_eq!(before.selected, after.selected);
    assert_eq!(before.tabs[0].entries[0].id, after.tabs[0].entries[0].id);
    router.shutdown();
    pool.run_until_stalled();
}

#[test]
fn terminal_failures_in_closed_or_nonempty_windows_do_not_exceed_capacity() {
    let mount: Mount<Destination> = Rc::new(|destination, _, _| {
        Box::pin(async move {
            if matches!(destination, Destination::Broken) {
                Err(MountError("capacity fixture".into()))
            } else {
                Ok(Rc::new(()) as Resource)
            }
        })
    });
    let router = Router::with_mount(
        RouterConfig {
            navigation_records: 1,
            ..RouterConfig::default()
        },
        Collector::default(),
        mount,
    );
    router
        .window_host(WindowHost(Rc::new(Cell::new(0))))
        .unwrap();
    let mut pool = LocalPool::new();
    let driven = router.clone();
    pool.spawner()
        .spawn_local(async move { driven.run().await.unwrap() })
        .unwrap();

    let opened = match pool.run_until(
        router
            .navigator()
            .open(Destination::Home, OpenTarget::NewWindow)
            .wait(),
    ) {
        Ok(Outcome::Committed(opened)) => opened,
        _ => panic!("new window must commit"),
    };
    let closed_window = opened.window_id;
    let closed_failure = router
        .navigator()
        .for_window(closed_window)
        .open(Destination::Broken, OpenTarget::NewTab);
    let closed_failure_id = closed_failure.id();
    assert!(matches!(
        pool.run_until(closed_failure.wait()),
        Err(OperationError::Mount(_))
    ));
    pool.run_until(router.close_window(closed_window).wait())
        .expect("close window");

    let current_failure = router
        .navigator()
        .open(Destination::Broken, OpenTarget::NewTab);
    let current_failure_id = current_failure.id();
    assert!(matches!(
        pool.run_until(current_failure.wait()),
        Err(OperationError::Mount(_))
    ));
    let statuses = router.snapshot().navigation;
    assert!(
        statuses
            .iter()
            .any(|status| status.key.operation_id == current_failure_id)
    );
    assert!(
        !statuses
            .iter()
            .any(|status| status.key.operation_id == closed_failure_id)
    );
}

#[test]
fn failed_mount_remains_keyed_and_retries_after_operation_retention_expires() {
    let fail = Rc::new(Cell::new(true));
    let observed = fail.clone();
    let mount: Mount<Destination> = Rc::new(move |destination, _, _| {
        let fail = observed.clone();
        Box::pin(async move {
            if matches!(destination, Destination::Broken) && fail.get() {
                Err(MountError("retry me".into()))
            } else {
                Ok(Rc::new(()) as Resource)
            }
        })
    });

    let router = Router::with_mount(
        RouterConfig {
            completed_operations: 0,
            ..RouterConfig::default()
        },
        Collector::default(),
        mount,
    );
    let mut pool = LocalPool::new();
    let driven = router.clone();
    pool.spawner()
        .spawn_local(async move { driven.run().await.unwrap() })
        .unwrap();

    let operation = router
        .navigator()
        .open(Destination::Broken, OpenTarget::NewTab);
    let id = operation.id();
    assert!(matches!(
        pool.run_until(operation.wait()),
        Err(OperationError::Mount(_))
    ));
    assert!(matches!(
        router.operation(id),
        Err(OperationError::OperationExpired)
    ));

    let key = router
        .navigation(crate::router::WindowId(1))
        .into_iter()
        .find(|status| status.key.operation_id == id)
        .expect("retained failed navigation")
        .key;

    let unrelated = router
        .navigator()
        .open(Destination::Other, OpenTarget::NewTab);
    assert!(matches!(
        pool.run_until(unrelated.wait()),
        Ok(Outcome::Committed(_))
    ));
    assert!(
        router
            .navigation(crate::router::WindowId(1))
            .iter()
            .any(|status| {
                status.key == key
                    && matches!(
                        &status.state,
                        NavigationState::Failed(OperationError::Mount(_))
                    )
            })
    );

    let foreign = Router::<Destination>::new(RouterConfig::default());
    assert!(matches!(
        foreign.retry_navigation(&key),
        Err(OperationError::UnknownOperation)
    ));

    fail.set(false);
    let retry = router.retry_navigation(&key).expect("retry");
    assert!(matches!(
        pool.run_until(retry.wait()),
        Ok(Outcome::Committed(_))
    ));
    assert!(
        router
            .navigation(crate::router::WindowId(1))
            .iter()
            .all(|status| { status.key.operation_id != id })
    );
}

#[test]
fn pending_retry_is_deduplicated_and_dismissing_child_removes_ancestor_failure() {
    let mode = Rc::new(Cell::new(0_u8));
    let observed = mode.clone();
    let mount: Mount<Destination> = Rc::new(move |destination, _, _| {
        let mode = observed.clone();
        Box::pin(async move {
            if matches!(destination, Destination::Broken) {
                match mode.get() {
                    0 => Err(MountError("first failure".into())),
                    1 => {
                        futures::future::pending::<()>().await;
                        unreachable!("pending mount resumed")
                    }
                    _ => Ok(Rc::new(()) as Resource),
                }
            } else {
                Ok(Rc::new(()) as Resource)
            }
        })
    });
    let router = Router::with_mount(RouterConfig::default(), Collector::default(), mount);
    let mut pool = LocalPool::new();
    let driven = router.clone();
    pool.spawner()
        .spawn_local(async move { driven.run().await.unwrap() })
        .unwrap();

    let first = router
        .navigator()
        .open(Destination::Broken, OpenTarget::NewTab);
    let first_id = first.id();
    assert!(matches!(
        pool.run_until(first.wait()),
        Err(OperationError::Mount(_))
    ));
    let first_key = router
        .navigation(crate::router::WindowId(1))
        .into_iter()
        .find(|status| status.key.operation_id == first_id)
        .expect("first failure")
        .key;

    mode.set(1);
    let retry = router.retry_navigation(&first_key).expect("retry");
    let retry_id = retry.id();
    pool.run_until_stalled();
    assert!(matches!(
        router.retry_navigation(&first_key),
        Err(OperationError::InvalidOptions)
    ));
    assert!(router.cancel(retry_id).unwrap());
    assert!(matches!(
        pool.run_until(retry.wait()),
        Ok(Outcome::Cancelled(crate::router::CancelReason::Requested))
    ));

    let retry_key = router
        .navigation(crate::router::WindowId(1))
        .into_iter()
        .find(|status| status.key.operation_id == retry_id)
        .expect("cancelled retry")
        .key;
    router
        .dismiss_navigation(&retry_key)
        .expect("dismiss retry chain");
    assert!(
        router
            .navigation(crate::router::WindowId(1))
            .iter()
            .all(
                |status| status.key.operation_id != first_id && status.key.operation_id != retry_id
            )
    );
}

#[test]
fn history_reuses_local_resources_and_failed_mounts_leave_the_old_view() {
    let mounted = Rc::new(RefCell::new(Vec::new()));
    let observed = mounted.clone();

    let mount: Mount<Destination> = Rc::new(move |destination, _, _| {
        let observed = observed.clone();

        Box::pin(async move {
            if matches!(destination, Destination::Broken) {
                return Err(MountError("fixture rejects mount".into()));
            }

            let draft = Rc::new(Cell::new(0_u32));

            observed.borrow_mut().push(draft.clone());

            Ok(draft as Resource)
        })
    });

    let router = Router::with_mount(RouterConfig::default(), Collector::new(100), mount);
    let mut pool = LocalPool::new();
    let driven = router.clone();

    pool.spawner()
        .spawn_local(async move {
            driven.run().await.expect("drive");
        })
        .expect("spawn");
    let a = match pool
        .run_until(
            router
                .navigator()
                .open(Destination::Home, OpenTarget::NewTab)
                .wait(),
        )
        .expect("open")
    {
        Outcome::Committed(opened) => opened,
        _ => panic!("open must commit"),
    };

    mounted.borrow()[0].set(42);
    let tab = router.navigator().tab(a.tab_id);

    pool.run_until(tab.navigate(Destination::Other).wait())
        .expect("navigate");
    pool.run_until(tab.back().wait()).expect("back");

    assert_eq!(
        mounted.borrow().len(),
        2,
        "back must not reconstruct a screen"
    );
    assert_eq!(mounted.borrow()[0].get(), 42);
    let error = pool.run_until(tab.replace(Destination::Broken).wait());

    assert!(matches!(error, Err(OperationError::Mount(_))));
    let snapshot = router.snapshot();

    assert_eq!(snapshot.tabs[0].entries.len(), 2);
    assert_eq!(snapshot.tabs[0].entries[0].id, a.entry_id);
    assert_eq!(snapshot.tabs[0].cursor, 0);
    assert_eq!(
        snapshot.tabs[0].entries[0].destination,
        Destination::Home.encode()
    );
    assert_eq!(mounted.borrow().len(), 2);
}

#[test]
fn typed_admission_redirects_in_one_operation_and_reports_final_target() {
    let router = Router::new(RouterConfig::default());
    router
        .entry_admission(|context| async move {
            if matches!(context.destination, Destination::Other) {
                Admission::Allow
            } else {
                Admission::Redirect(Destination::Other)
            }
        })
        .unwrap();

    let mut pool = LocalPool::new();
    let driven = router.clone();
    pool.spawner()
        .spawn_local(async move { driven.run().await.unwrap() })
        .unwrap();

    let operation = router
        .navigator()
        .open(Destination::Home, OpenTarget::NewTab);
    let id = operation.id();
    let outcome = pool.run_until(operation.wait()).unwrap();

    assert!(matches!(outcome, Outcome::Committed(_)));
    let status = router
        .navigation(crate::router::WindowId(1))
        .into_iter()
        .find(|status| status.key.operation_id == id)
        .expect("navigation status");
    assert_eq!(status.key.destination, Destination::Other.encode());
    assert_eq!(status.state, NavigationState::Committed);
}

#[test]
fn invalid_redirect_is_rejected_before_mount_or_new_window_host_creation() {
    let mounts = Rc::new(Cell::new(0));
    let observed = mounts.clone();
    let mount: Mount<Destination> = Rc::new(move |_, _, _| {
        observed.set(observed.get() + 1);
        Box::pin(async { Ok(Rc::new(()) as Resource) })
    });
    let router = Router::with_mount(RouterConfig::default(), Collector::default(), mount);
    let opens = Rc::new(Cell::new(0));
    let host = Rc::new(Cell::new(0));
    router
        .window_host(CountingHost {
            opened: opens.clone(),
            closed: host,
        })
        .unwrap();
    router
        .entry_admission(|context| async move {
            if matches!(context.destination, Destination::Home) {
                Admission::Redirect(Destination::Item(".".into()))
            } else {
                Admission::Allow
            }
        })
        .unwrap();

    let mut pool = LocalPool::new();
    let driven = router.clone();
    pool.spawner()
        .spawn_local(async move { driven.run().await.unwrap() })
        .unwrap();
    let operation = router
        .navigator()
        .open(Destination::Home, OpenTarget::NewWindow);
    assert!(matches!(
        pool.run_until(operation.wait()),
        Err(OperationError::Route(_))
    ));
    assert_eq!(mounts.get(), 0);
    assert_eq!(opens.get(), 0);
}

#[test]
fn redirect_loop_is_bounded_and_cancellable_admission_does_not_mount() {
    let router = Router::new(RouterConfig {
        max_admission_redirects: 2,
        ..RouterConfig::default()
    });
    router
        .entry_admission(|_| async { Admission::Redirect(Destination::Home) })
        .unwrap();

    let mut pool = LocalPool::new();
    let driven = router.clone();
    pool.spawner()
        .spawn_local(async move { driven.run().await.unwrap() })
        .unwrap();

    let operation = router
        .navigator()
        .open(Destination::Home, OpenTarget::NewTab);
    assert!(matches!(
        pool.run_until(operation.wait()),
        Err(OperationError::AdmissionRedirectLoop { limit: 2 })
    ));

    router.shutdown();
    pool.run_until_stalled();
}

#[test]
fn cancellation_drops_pending_admission_and_retains_cancelled_status() {
    let router = Router::new(RouterConfig::default());
    router
        .entry_admission(|_| async { futures::future::pending::<Admission<Destination>>().await })
        .unwrap();

    let mut pool = LocalPool::new();
    let driven = router.clone();
    pool.spawner()
        .spawn_local(async move { driven.run().await.unwrap() })
        .unwrap();

    let operation = router
        .navigator()
        .open(Destination::Home, OpenTarget::NewTab);
    let id = operation.id();
    pool.run_until_stalled();
    assert!(router.cancel(id).unwrap());

    assert_eq!(
        pool.run_until(operation.wait()).unwrap(),
        Outcome::Cancelled(crate::router::CancelReason::Requested)
    );
    assert!(
        router
            .navigation(crate::router::WindowId(1))
            .iter()
            .any(|status| {
                status.key.operation_id == id
                    && matches!(
                        &status.state,
                        NavigationState::Cancelled(crate::router::CancelReason::Requested)
                    )
            })
    );
}

#[test]
fn pending_navigation_records_are_not_trimmed_by_terminal_capacity() {
    let router = Router::new(RouterConfig {
        navigation_records: 1,
        ..RouterConfig::default()
    });
    router
        .entry_admission(|_| async { futures::future::pending::<Admission<Destination>>().await })
        .unwrap();

    let mut pool = LocalPool::new();
    let driven = router.clone();
    pool.spawner()
        .spawn_local(async move { driven.run().await.unwrap() })
        .unwrap();
    let first = router
        .navigator()
        .open(Destination::Home, OpenTarget::NewTab);
    let second = router
        .navigator()
        .open(Destination::Other, OpenTarget::NewTab);
    let first_id = first.id();
    let second_id = second.id();
    pool.run_until_stalled();

    assert_eq!(router.navigation(crate::router::WindowId(1)).len(), 2);
    assert!(router.cancel(first_id).unwrap());
    assert!(matches!(
        pool.run_until(first.wait()),
        Ok(Outcome::Cancelled(crate::router::CancelReason::Requested))
    ));
    assert!(
        router
            .navigation(crate::router::WindowId(1))
            .iter()
            .any(|status| {
                status.key.operation_id == second_id
                    && matches!(&status.state, NavigationState::Pending)
            })
    );
    assert!(router.cancel(second_id).unwrap());
    assert!(matches!(
        pool.run_until(second.wait()),
        Ok(Outcome::Cancelled(crate::router::CancelReason::Requested))
    ));
    assert_eq!(router.navigation(crate::router::WindowId(1)).len(), 1);
}

#[test]
fn failed_browser_restore_keeps_old_entry_and_reconciles_old_url() {
    let mount: Mount<Destination> = Rc::new(|destination, _, _| {
        Box::pin(async move {
            if matches!(destination, Destination::Broken) {
                Err(MountError("restore failed".into()))
            } else {
                Ok(Rc::new(()) as Resource)
            }
        })
    });
    let router = Router::with_mount(RouterConfig::default(), Collector::default(), mount);
    let history = Rc::new(RefCell::new(Vec::new()));
    let observed = history.clone();
    router.on_history(move |change| {
        observed.borrow_mut().push(change);
        Ok(())
    });

    let mut pool = LocalPool::new();
    let driven = router.clone();
    pool.spawner()
        .spawn_local(async move { driven.run().await.unwrap() })
        .unwrap();

    let opened = match pool
        .run_until(
            router
                .navigator()
                .open(Destination::Home, OpenTarget::NewTab)
                .wait(),
        )
        .unwrap()
    {
        Outcome::Committed(opened) => opened,
        _ => panic!("home must commit"),
    };
    let checkpoint = BrowserCheckpoint {
        version: 1,
        runtime_id: router.runtime_id(),
        tab_id: opened.tab_id,
        entry_id: opened.entry_id,
        destination: Destination::Broken.encode(),
    };

    assert!(matches!(
        pool.run_until(router.navigator().restore(checkpoint).wait()),
        Err(OperationError::Mount(_))
    ));
    assert_eq!(
        router.snapshot().tabs[0].entries[0].destination,
        Destination::Home.encode()
    );
    assert!(matches!(
        history.borrow().last(),
        Some(HistoryChange::Replace(_, uri)) if uri == "/"
    ));
}

#[test]
fn redirected_browser_restore_replaces_history_with_final_destination() {
    let router = Router::new(RouterConfig::default());
    router
        .entry_admission(|context| async move {
            if matches!(context.destination, Destination::Broken) {
                Admission::Redirect(Destination::Other)
            } else {
                Admission::Allow
            }
        })
        .unwrap();
    let history = Rc::new(RefCell::new(Vec::new()));
    let observed = history.clone();
    router.on_history(move |change| {
        observed.borrow_mut().push(change);
        Ok(())
    });

    let mut pool = LocalPool::new();
    let driven = router.clone();
    pool.spawner()
        .spawn_local(async move { driven.run().await.unwrap() })
        .unwrap();
    let opened = match pool.run_until(
        router
            .navigator()
            .open(Destination::Home, OpenTarget::NewTab)
            .wait(),
    ) {
        Ok(Outcome::Committed(opened)) => opened,
        _ => panic!("home must commit"),
    };
    let checkpoint = BrowserCheckpoint {
        version: 1,
        runtime_id: router.runtime_id(),
        tab_id: opened.tab_id,
        entry_id: opened.entry_id,
        destination: Destination::Broken.encode(),
    };

    assert!(matches!(
        pool.run_until(router.navigator().restore(checkpoint).wait()),
        Ok(Outcome::Committed(_))
    ));
    let history = history.borrow();
    assert_eq!(history.len(), 2);
    assert!(matches!(
        history.last(),
        Some(HistoryChange::Replace(_, uri)) if uri == "/other"
    ));
    let status = router
        .navigation(crate::router::WindowId(1))
        .into_iter()
        .find(|status| status.key.destination == Destination::Other.encode())
        .expect("final restore status");
    assert_eq!(status.state, NavigationState::Committed);
}

#[test]
fn retrying_restore_after_browser_epoch_changes_is_rejected_as_stale() {
    let mount: Mount<Destination> = Rc::new(|destination, _, _| {
        Box::pin(async move {
            if matches!(destination, Destination::Broken) {
                Err(MountError("restore retry fixture".into()))
            } else {
                Ok(Rc::new(()) as Resource)
            }
        })
    });
    let router = Router::with_mount(RouterConfig::default(), Collector::default(), mount);
    let mut pool = LocalPool::new();
    let driven = router.clone();
    pool.spawner()
        .spawn_local(async move { driven.run().await.unwrap() })
        .unwrap();
    let opened = match pool.run_until(
        router
            .navigator()
            .open(Destination::Home, OpenTarget::NewTab)
            .wait(),
    ) {
        Ok(Outcome::Committed(opened)) => opened,
        _ => panic!("home must commit"),
    };
    let checkpoint = BrowserCheckpoint {
        version: 1,
        runtime_id: router.runtime_id(),
        tab_id: opened.tab_id,
        entry_id: opened.entry_id,
        destination: Destination::Broken.encode(),
    };
    assert!(matches!(
        pool.run_until(router.navigator().restore(checkpoint).wait()),
        Err(OperationError::Mount(_))
    ));
    let key = router
        .navigation(crate::router::WindowId(1))
        .into_iter()
        .find(|status| status.key.destination == Destination::Broken.encode())
        .expect("failed restore status")
        .key;

    assert!(matches!(
        pool.run_until(
            router
                .navigator()
                .open(Destination::Other, OpenTarget::NewTab)
                .wait()
        ),
        Ok(Outcome::Committed(_))
    ));
    assert!(matches!(
        router.retry_navigation(&key),
        Err(OperationError::StaleNavigation)
    ));
}

#[test]
fn mount_registration_and_failed_mounts_follow_committed_entry_lifetimes() {
    let runs = Rc::new(Cell::new(0));
    let registered = Rc::new(RefCell::new(Vec::new()));
    let started = runs.clone();
    let observed = registered.clone();
    let app_tasks = Rc::new(RefCell::new(Vec::new()));
    let app_observed = app_tasks.clone();

    let mount: Mount<Destination> = Rc::new(move |destination, entry, navigator| {
        let started = started.clone();
        let observed = observed.clone();
        let app_observed = app_observed.clone();

        Box::pin(async move {
            let task = entry
                .activity("load", move |_| {
                    started.set(started.get() + 1);

                    async {
                        futures::future::pending::<()>().await;

                        Ok(())
                    }
                })
                .expect("register while mounting");

            observed.borrow_mut().push(task);

            if matches!(destination, Destination::Broken) {
                return Err(MountError("fixture".into()));
            }

            let app = navigator
                .app()
                .tasks()
                .spawn("export", async {
                    futures::future::pending::<()>().await;

                    Ok(())
                })
                .expect("app scope");

            app_observed.borrow_mut().push(app);
            navigator
                .tab(entry.tab_id().unwrap())
                .tasks()
                .spawn("mounted tab work", async { Ok(()) })
                .expect("provisional tab scope is available");

            Ok(Rc::new(()) as Resource)
        })
    });

    let router = Router::with_mount(RouterConfig::default(), Collector::default(), mount);
    let mut pool = LocalPool::new();
    let driver = router.clone();

    pool.spawner()
        .spawn_local(async move {
            driver.run().await.unwrap();
        })
        .unwrap();
    let opened = match pool
        .run_until(
            router
                .navigator()
                .open(Destination::Home, OpenTarget::NewTab)
                .wait(),
        )
        .unwrap()
    {
        Outcome::Committed(opened) => opened,
        _ => panic!("commit"),
    };

    pool.run_until_stalled();
    assert_eq!(runs.get(), 1);
    assert!(matches!(
        pool.run_until(
            router
                .navigator()
                .tab(opened.tab_id)
                .replace(Destination::Broken)
                .wait()
        ),
        Err(OperationError::Mount(_))
    ));
    pool.run_until_stalled();
    assert_eq!(runs.get(), 1, "uncommitted entry activities must never run");
    assert_eq!(
        registered.borrow()[0].snapshot().unwrap().state,
        crate::tasks::TaskState::Running
    );
    assert_eq!(
        registered.borrow()[1].snapshot().unwrap().state,
        crate::tasks::TaskState::Cancelled
    );
    pool.run_until(router.navigator().tab(opened.tab_id).close().wait())
        .unwrap();
    pool.run_until_stalled();
    let export = app_tasks.borrow()[0].snapshot().unwrap();

    assert_eq!(export.state, crate::tasks::TaskState::Running);
    assert_eq!(export.origin.entry_id, Some(opened.entry_id));
    assert_eq!(export.origin.tab_id, Some(opened.tab_id));
}
