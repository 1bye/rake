//! Public API coverage for typed admission and keyed navigation status.

use futures::executor::LocalPool;
use futures::task::LocalSpawnExt;
use rake::prelude::*;
use rake::router::{CancelReason, NavigationState, WindowId};

#[derive(Clone, Debug, PartialEq, Eq, rake::Route)]
enum Destination {
    #[route(id = "home", path = "/")]
    Home,
    #[route(id = "login", path = "/login")]
    Login,
}

fn harness(config: RouterConfig) -> (Router<Destination>, LocalPool) {
    let router = Router::new(config);
    let pool = LocalPool::new();
    let driven = router.clone();

    pool.spawner()
        .spawn_local(async move { driven.run().await.unwrap() })
        .unwrap();

    (router, pool)
}

#[test]
fn public_admission_redirect_keeps_one_operation_and_final_status() {
    let (router, mut pool) = harness(RouterConfig::default());
    router
        .entry_admission(|context| async move {
            if matches!(context.destination, Destination::Login) {
                Admission::Allow
            } else {
                Admission::Redirect(Destination::Login)
            }
        })
        .unwrap();

    let operation = router
        .navigator()
        .open(Destination::Home, OpenTarget::NewTab);
    let id = operation.id();
    assert!(matches!(
        pool.run_until(operation.wait()),
        Ok(Outcome::Committed(_))
    ));

    let status = router
        .navigation(WindowId(1))
        .into_iter()
        .find(|status| status.key.operation_id == id)
        .expect("status retained");
    assert_eq!(status.key.destination, Destination::Login.encode());
    assert_eq!(status.state, NavigationState::Committed);
}

#[test]
fn public_pending_admission_can_be_cancelled_by_operation_id() {
    let (router, mut pool) = harness(RouterConfig::default());
    router
        .entry_admission(|_| async { futures::future::pending::<Admission<Destination>>().await })
        .unwrap();

    let operation = router
        .navigator()
        .open(Destination::Home, OpenTarget::NewTab);
    let id = operation.id();
    pool.run_until_stalled();
    assert!(router.cancel(id).unwrap());
    assert_eq!(
        pool.run_until(operation.wait()).unwrap(),
        Outcome::Cancelled(CancelReason::Requested)
    );
}

#[test]
fn public_router_snapshot_exposes_keyed_navigation() {
    let (router, mut pool) = harness(RouterConfig::default());
    let operation = router
        .navigator()
        .open(Destination::Home, OpenTarget::NewTab);
    let id = operation.id();
    pool.run_until(operation.wait()).unwrap();

    let status = router
        .snapshot()
        .navigation
        .into_iter()
        .find(|status| status.key.operation_id == id)
        .expect("snapshot status");
    assert_eq!(status.key.window_id, WindowId(1));
    assert_eq!(status.key.destination, Destination::Home.encode());
}
