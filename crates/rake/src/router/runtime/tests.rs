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
use crate::router::{MountError, OpenTarget, OperationError, Outcome, Route, Router, RouterConfig};

#[derive(Clone, crate::Route)]
enum Destination {
    #[route(id = "home", path = "/")]
    Home,
    #[route(id = "other", path = "/other")]
    Other,
    #[route(id = "broken", path = "/broken")]
    Broken,
}

struct WindowHost(Rc<Cell<usize>>);

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
