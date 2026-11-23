//! Covers: the public route mount contract through navigation and workspace restore.
//! Real: local router, guards, history and checkpoint APIs.
//! Doubles: retained mounts recording presentation and disposal.
//! Requires: no UI framework.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use futures::executor::LocalPool;
use futures::future::{LocalBoxFuture, pending};
use futures::task::LocalSpawnExt as _;
use rake::http::CancellationToken;
use rake::reload::{ReloadMechanism, ReloadPhase, ReloadRequest, ReloadState};
use rake::router::{
    Activation, CancelReason, EntryId, GuardReason, GuardResult, MountError, MountedRoute,
    OpenTarget, Operation, Outcome, PreparedPage, RouteMount, RouteMountContext, RoutePresentation,
    Router, RouterConfig, WindowHost, WindowId,
};
use rake::services::ServiceRegistry;
use rake::tasks::HostVisibility;
use serde::{Deserialize, Serialize};

const APP_ID: &str = "test.route-mount";

#[derive(Clone, Copy, Debug, Eq, PartialEq, rake::Route)]
enum Destination {
    #[route(id = "home", path = "/")]
    Home,
    #[route(id = "detail", path = "/detail")]
    Detail,
    #[route(id = "denied", path = "/denied")]
    Denied,
    #[route(id = "slow", path = "/slow")]
    Slow,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Saved(u32);

impl ReloadState for Saved {
    const SCHEMA: &'static str = "test.view";
    const VERSION: u32 = 1;
}

#[derive(Debug, PartialEq)]
enum Event {
    Prepared(Destination),
    PrepareDropped,
    Mounted(Destination, EntryId, Option<u32>),
    Opened(WindowId),
    Presented(EntryId, RoutePresentation),
    Disposed(EntryId),
    Closed(WindowId),
}

type Events = Rc<RefCell<Vec<Event>>>;
type Probe = dyn Fn() -> (ReloadPhase, usize);

struct Adapter {
    events: Events,
    load_token: Rc<RefCell<Option<CancellationToken>>>,
    probe: Option<Weak<Probe>>,
    skip_restore: Option<Destination>,
}

struct PendingPreparation(Events);

impl Drop for PendingPreparation {
    fn drop(&mut self) {
        self.0.borrow_mut().push(Event::PrepareDropped);
    }
}

impl RouteMount<Destination> for Adapter {
    fn prepare(
        &self,
        destination: &Destination,
        services: ServiceRegistry,
        cancellation: CancellationToken,
    ) -> LocalBoxFuture<'static, Result<PreparedPage, MountError>> {
        let destination = *destination;
        assert_eq!(*services.get::<u32>().expect("route service"), 42);
        self.events.borrow_mut().push(Event::Prepared(destination));

        if destination == Destination::Slow {
            *self.load_token.borrow_mut() = Some(cancellation);
            let preparation = PendingPreparation(self.events.clone());

            return Box::pin(async move {
                let _preparation = preparation;
                pending().await
            });
        }

        Box::pin(async move { Ok(PreparedPage::from_value(destination)) })
    }

    fn mount(
        &self,
        destination: &Destination,
        prepared: PreparedPage,
        context: RouteMountContext<Destination>,
    ) -> LocalBoxFuture<'static, Result<Box<dyn MountedRoute>, MountError>> {
        let destination = *destination;
        let events = self.events.clone();
        let probe = self.probe.clone();
        let skip_restore = self.skip_restore == Some(destination);

        Box::pin(async move {
            assert_eq!(prepared.downcast::<Destination>()?, destination);
            assert_eq!(
                context.entry.id().expect("provisional entry"),
                context.entry_id
            );
            assert_eq!(
                context.entry.window_id().expect("provisional window"),
                context.window_id
            );

            if let Some(probe) = probe.as_ref().and_then(Weak::upgrade) {
                assert_eq!(probe(), (ReloadPhase::Restoring, 0));
            }

            let saved = if skip_restore {
                None
            } else {
                context
                    .entry
                    .reload_state::<Saved>()
                    .expect("read checkpoint")
            };
            let value = saved.as_ref().map(|saved| saved.0);
            events
                .borrow_mut()
                .push(Event::Mounted(destination, context.entry_id, value));

            if destination == Destination::Denied {
                // The mount revises the value checked by the initial enter guard.
                context
                    .entry
                    .set_dirty(true)
                    .expect("revise provisional entry");
            }

            let initial = if destination == Destination::Detail {
                11
            } else {
                7
            };
            context
                .entry
                .on_reload(move || Saved(value.unwrap_or(initial)).checkpoint())
                .expect("register entry checkpoint");

            Ok(Box::new(View {
                events,
                context,
                probe,
            }) as Box<dyn MountedRoute>)
        })
    }
}

struct View {
    events: Events,
    context: RouteMountContext<Destination>,
    probe: Option<Weak<Probe>>,
}

impl MountedRoute for View {
    fn presentation_changed(&self, presentation: RoutePresentation) {
        if let Some(probe) = self.probe.as_ref().and_then(Weak::upgrade) {
            assert_eq!(probe(), (ReloadPhase::Restoring, 2));
        }

        self.events
            .borrow_mut()
            .push(Event::Presented(self.context.entry_id, presentation));
    }

    fn dispose(&self) {
        assert!(
            self.context.entry.id().is_err(),
            "dispose must invalidate weak handles"
        );
        self.events
            .borrow_mut()
            .push(Event::Disposed(self.context.entry_id));
    }
}

struct TestWindow(Events);

impl WindowHost for TestWindow {
    fn open(&self, id: WindowId, _: Activation) -> LocalBoxFuture<'static, Result<(), MountError>> {
        self.0.borrow_mut().push(Event::Opened(id));

        Box::pin(async { Ok(()) })
    }

    fn close(&self, id: WindowId) {
        self.0.borrow_mut().push(Event::Closed(id));
    }

    fn activate(&self, _: WindowId) {}
}

fn router(events: Events, token: Rc<RefCell<Option<CancellationToken>>>) -> Router<Destination> {
    with_adapter(Adapter {
        events,
        load_token: token,
        probe: None,
        skip_restore: None,
    })
}

fn with_adapter(adapter: Adapter) -> Router<Destination> {
    let services = ServiceRegistry::new()
        .with(42u32)
        .expect("register route service");

    Router::with_route_mount(RouterConfig::default(), services, adapter)
}

fn drive(pool: &LocalPool, router: &Router<Destination>) {
    let router = router.clone();

    pool.spawner()
        .spawn_local(async move { router.run().await.expect("drive router") })
        .expect("spawn driver");
}

async fn committed<T: Clone>(operation: Operation<T>) -> T {
    match operation.wait().await.expect("route operation completed") {
        Outcome::Committed(result) => result,
        _ => panic!("expected a committed operation"),
    }
}

fn take(events: &Events) -> Vec<Event> {
    std::mem::take(&mut *events.borrow_mut())
}

#[test]
fn transfers_prepared_data_and_preserves_mounts_through_history() {
    use Destination::{Detail, Home};
    use Event::{Disposed, Mounted, Prepared, Presented};
    use RoutePresentation::{Presented as Selected, Retained};

    let events = Events::default();
    let router = router(events.clone(), Default::default());
    router
        .window_host(TestWindow(events.clone()))
        .expect("install window host");
    let mut pool = LocalPool::new();
    drive(&pool, &router);

    pool.run_until(async {
        let nav = router.navigator();
        let home = committed(nav.open(Home, OpenTarget::NewTab)).await;
        let tab = nav.tab(home.tab_id);
        let detail = committed(tab.navigate(Detail)).await;
        router
            .set_window_visibility(home.window_id, HostVisibility::Hidden)
            .expect("hide window");
        router
            .set_window_visibility(home.window_id, HostVisibility::Visible)
            .expect("show window");

        assert_eq!(committed(tab.back()).await.entry_id, home.entry_id);
        assert_eq!(committed(tab.forward()).await.entry_id, detail.entry_id);
        assert_eq!(committed(tab.back()).await.entry_id, home.entry_id);

        let new = committed(tab.navigate(Detail)).await;
        assert_ne!(new.entry_id, detail.entry_id);
        committed(router.close_window(home.window_id)).await;
        router.shutdown();
        router.shutdown();

        let unknown = Selected(HostVisibility::Unknown);
        let hidden = Selected(HostVisibility::Hidden);
        let visible = Selected(HostVisibility::Visible);
        assert_eq!(
            take(&events),
            [
                Prepared(Home),
                Mounted(Home, home.entry_id, None),
                Presented(home.entry_id, unknown),
                Prepared(Detail),
                Mounted(Detail, detail.entry_id, None),
                Presented(home.entry_id, Retained),
                Presented(detail.entry_id, unknown),
                Presented(detail.entry_id, hidden),
                Presented(detail.entry_id, visible),
                Presented(detail.entry_id, Retained),
                Presented(home.entry_id, visible),
                Presented(home.entry_id, Retained),
                Presented(detail.entry_id, visible),
                Presented(detail.entry_id, Retained),
                Presented(home.entry_id, visible),
                Prepared(Detail),
                Mounted(Detail, new.entry_id, None),
                Disposed(detail.entry_id),
                Presented(home.entry_id, Retained),
                Presented(new.entry_id, visible),
                Disposed(home.entry_id),
                Presented(new.entry_id, Retained),
                Disposed(new.entry_id),
                Event::Closed(home.window_id),
            ]
        );
    });
    pool.run();
    assert!(take(&events).is_empty());
}

#[test]
fn cancellation_during_prepare_drops_the_future_without_mounting() {
    let events = Events::default();
    let token = Rc::default();
    let router = router(events.clone(), Rc::clone(&token));
    let mut pool = LocalPool::new();
    drive(&pool, &router);

    let open = router
        .navigator()
        .open(Destination::Slow, OpenTarget::NewTab);
    pool.run_until_stalled();

    assert_eq!(take(&events), [Event::Prepared(Destination::Slow)]);
    assert!(router.cancel(open.id()).expect("cancel prepare"));
    assert!(matches!(
        pool.run_until(open.wait()),
        Ok(Outcome::Cancelled(CancelReason::Requested))
    ));
    assert!(token.borrow().as_ref().expect("load token").is_cancelled());
    assert_eq!(take(&events), [Event::PrepareDropped]);
    assert!(router.snapshot().tabs.is_empty());
    router.shutdown();
    pool.run();
}

#[test]
fn guard_revalidation_rolls_back_a_mount_and_shutdown_disposes_the_committed_entry_once() {
    let events = Events::default();
    let router = router(events.clone(), Default::default());
    router
        .window_host(TestWindow(events.clone()))
        .expect("install window host");
    router
        .entry_guard(|context| async move {
            if context.destination == Destination::Denied && context.dirty {
                GuardResult::Deny(GuardReason("revision denied".into()))
            } else {
                GuardResult::Allow
            }
        })
        .expect("install guard");

    let mut pool = LocalPool::new();
    drive(&pool, &router);

    pool.run_until(async {
        let nav = router.navigator();
        let home = committed(nav.open(Destination::Home, OpenTarget::NewTab)).await;
        take(&events);
        let outcome = nav
            .open(Destination::Denied, OpenTarget::NewWindow)
            .wait()
            .await
            .expect("guard outcome");
        assert!(matches!(outcome, Outcome::Blocked(_)));

        let rollback = take(&events);
        let [
            Event::Prepared(Destination::Denied),
            Event::Opened(window),
            Event::Mounted(Destination::Denied, denied, None),
            Event::Disposed(disposed),
            Event::Closed(closed),
        ] = rollback.as_slice()
        else {
            panic!("expected a mounted then rolled-back entry, got {rollback:?}");
        };
        assert_eq!(denied, disposed);
        assert_eq!(window, closed);
        assert!(router.entry(*denied).is_err());
        assert!(router.entry(home.entry_id).is_ok());
        assert_eq!(router.snapshot().tabs[0].entries.len(), 1);

        router.shutdown();
        router.shutdown();
        assert_eq!(
            take(&events),
            [
                Event::Presented(home.entry_id, RoutePresentation::Retained),
                Event::Disposed(home.entry_id),
            ]
        );
    });
    pool.run();
    assert!(take(&events).is_empty());
}

#[test]
fn reload_consumes_provisional_checkpoints_and_becomes_ready_after_attachment() {
    let mut pool = LocalPool::new();
    let old = router(Events::default(), Default::default());
    drive(&pool, &old);

    let (home, detail, checkpoint) = pool.run_until(async {
        let nav = old.navigator();
        let home = committed(nav.open(Destination::Home, OpenTarget::NewTab)).await;
        let detail = committed(nav.tab(home.tab_id).navigate(Destination::Detail)).await;
        old.prepare_reload(ReloadRequest {
            id: "new-build".into(),
            mechanism: ReloadMechanism::NativeRestart,
        })
        .expect("prepare checkpoint");

        (
            home.entry_id,
            detail.entry_id,
            old.reload_checkpoint(APP_ID).expect("capture workspace"),
        )
    });

    let events = Events::default();
    let probe = Rc::new(RefCell::new(None::<Router<Destination>>));
    let inspect: Rc<Probe> = Rc::new({
        let probe = probe.clone();

        move || {
            let router = probe.borrow();
            let router = router.as_ref().expect("replacement router");
            let entry_count = router
                .snapshot()
                .tabs
                .iter()
                .map(|tab| tab.entries.len())
                .sum();

            (router.reload_status().phase, entry_count)
        }
    });
    let replacement = with_adapter(Adapter {
        events: events.clone(),
        load_token: Default::default(),
        probe: Some(Rc::downgrade(&inspect)),
        skip_restore: None,
    });
    *probe.borrow_mut() = Some(replacement.clone());
    pool.run_until(replacement.restore_workspace(&checkpoint, APP_ID))
        .expect("restore workspace");
    assert_eq!(replacement.reload_status().phase, ReloadPhase::Restored);
    assert_eq!(
        take(&events),
        [
            Event::Prepared(Destination::Home),
            Event::Prepared(Destination::Detail),
            Event::Mounted(Destination::Home, home, Some(7)),
            Event::Mounted(Destination::Detail, detail, Some(11)),
            Event::Presented(home, RoutePresentation::Retained),
            Event::Presented(
                detail,
                RoutePresentation::Presented(HostVisibility::Unknown)
            ),
        ]
    );
    drop(inspect);

    let broken = with_adapter(Adapter {
        events: events.clone(),
        load_token: Default::default(),
        probe: None,
        skip_restore: Some(Destination::Detail),
    });
    assert!(
        pool.run_until(broken.restore_workspace(&checkpoint, APP_ID))
            .is_err()
    );
    assert_eq!(broken.reload_status().phase, ReloadPhase::RestoreFailed);
    assert!(broken.snapshot().tabs.is_empty());
    let failed = take(&events);
    assert_eq!(
        &failed[..4],
        [
            Event::Prepared(Destination::Home),
            Event::Prepared(Destination::Detail),
            Event::Mounted(Destination::Home, home, Some(7)),
            Event::Mounted(Destination::Detail, detail, None),
        ]
    );
    assert_eq!(
        &failed[4..],
        [Event::Disposed(detail), Event::Disposed(home)]
    );

    old.shutdown();
    replacement.shutdown();
    pool.run();
}
