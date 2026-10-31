//! Covers: typed asynchronous route preparation, mixed loaded/plain route
//! registration, cancellation before entity construction, failed-load rollback,
//! retry, retained history and prepared-data type safety.
//! Real: RouteViews preparation/mount bridge, GPUI entities and router driver.
//! Doubles: GPUI TestPlatform; native window observation is intentionally omitted.
//! Requires: view-test-support; no display or native host.
#![cfg(all(feature = "view-test-support", not(target_family = "wasm")))]

use crate::diagnostics::Collector;
use crate::gpui::{self, Context, Entity, Render, Window, prelude::*};
use crate::http::{HttpClient, HttpConfig};
use crate::loading::LoadContext;
use crate::router::runtime::{Prepare, PreparedMount, Resource};
use crate::router::{
    LoadedPage, MountError, NavigationState, OpenTarget, OperationError, Outcome, Page,
    PageContext, PreparedPage, RouteViews, Router, RouterConfig, WindowId,
};
use crate::services::{ServiceRegistry, Services};
use futures::FutureExt as _;
use futures::channel::oneshot;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::rc::Rc;

#[derive(Clone, crate::Route)]
enum Destination {
    #[route(id = "home", path = "/home/{id}")]
    Home(u64),
    #[route(id = "other", path = "/other")]
    Other,
}

#[derive(Default)]
struct Probe {
    loads: Cell<usize>,
    mounts: Cell<usize>,
    loaded_ids: RefCell<Vec<u64>>,
    mounted_ids: RefCell<Vec<u64>>,
    fail_once: Cell<bool>,
    pending_gate: RefCell<Option<oneshot::Receiver<()>>>,
}

struct LoadedData {
    id: u64,
    // Deliberately neither Clone nor Send: the public loaded-page API must not
    // impose either bound on prepared route data.
    marker: Rc<Cell<usize>>,
}

#[derive(Debug)]
struct LoadFailure(&'static str);

impl Display for LoadFailure {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}

impl Error for LoadFailure {}

struct LoadedHome {
    id: u64,
}

impl LoadedPage<Destination> for LoadedHome {
    type Data = LoadedData;
    type Error = LoadFailure;

    async fn load(
        destination: Destination,
        context: LoadContext,
    ) -> Result<Self::Data, Self::Error> {
        let Destination::Home(id) = destination else {
            return Err(LoadFailure("loaded destination mismatch"));
        };
        let probe = context
            .services()
            .get::<Probe>()
            .map_err(|_| LoadFailure("probe service missing"))?;
        probe.loads.set(probe.loads.get() + 1);
        let gate = probe.pending_gate.borrow_mut().take();
        let fail = id == 99 && probe.fail_once.replace(false);

        if let Some(gate) = gate {
            let _ = gate.await;
        }
        if fail {
            return Err(LoadFailure("loaded page unavailable"));
        }

        probe.loaded_ids.borrow_mut().push(id);
        Ok(LoadedData {
            id,
            marker: Rc::new(Cell::new(0)),
        })
    }

    fn mount(
        destination: &Destination,
        data: Self::Data,
        page: PageContext<Destination>,
        _: &mut Window,
        cx: &mut gpui::App,
    ) -> Result<Entity<Self>, MountError> {
        let Destination::Home(id) = destination else {
            return Err(MountError("loaded mount destination mismatch".into()));
        };
        if *id != data.id {
            return Err(MountError("loaded data parameter mismatch".into()));
        }
        let _marker = data.marker.get();
        let probe = page
            .services()
            .get::<Probe>()
            .map_err(|error| MountError(error.to_string()))?;
        probe.mounts.set(probe.mounts.get() + 1);
        probe.mounted_ids.borrow_mut().push(*id);
        Ok(cx.new(|_| Self { id: *id }))
    }
}

impl Render for LoadedHome {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::div().child(format!("home {}", self.id))
    }
}

struct OtherPage;

impl Page<Destination> for OtherPage {
    fn mount(
        destination: &Destination,
        _: PageContext<Destination>,
        _: &mut Window,
        cx: &mut gpui::App,
    ) -> Result<Entity<Self>, MountError> {
        if !matches!(destination, Destination::Other) {
            return Err(MountError("plain page destination mismatch".into()));
        }
        Ok(cx.new(|_| Self))
    }
}

impl Render for OtherPage {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::div().child("other")
    }
}

struct Host {
    router: Router<Destination>,
    _driver: gpui::Task<()>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        crate::router::RouteOutlet::new(self.router.clone())
    }
}

fn services(probe: Probe) -> (Services, Rc<Probe>) {
    let registry = ServiceRegistry::new().with(probe).unwrap();
    let probe = registry.get::<Probe>().unwrap();
    let services = Services::new(
        HttpClient::new(HttpConfig::new()).unwrap(),
        Collector::default(),
        registry,
    );

    (services, probe)
}

fn add_host(cx: &mut gpui::TestAppContext, services: Services) -> gpui::WindowHandle<Host> {
    cx.add_window(move |window, cx| {
        let contexts = Rc::new(RefCell::new(BTreeMap::from([(
            WindowId(1),
            window.to_async(cx),
        )])));
        let views = Rc::new(crate::routes! {
            Destination::Home(..) => loaded(LoadedHome),
            Destination::Other => OtherPage,
        });
        let background = cx.background_executor().clone();
        let prepare_views = views.clone();
        let prepare_services = services.clone();
        let prepare: Prepare<Destination> = Rc::new(move |destination, _, _, cancellation| {
            let views = prepare_views.clone();
            let context =
                LoadContext::new(prepare_services.clone(), cancellation, background.clone());
            Box::pin(async move { views.prepare(&destination, context).await })
        });

        let mount_contexts = contexts.clone();
        let mount_views = views.clone();
        let mount_services = services.clone();
        let mount: PreparedMount<Destination> =
            Rc::new(move |destination, entry, navigator, prepared| {
                let context = mount_contexts.borrow().get(&navigator.window_id()).cloned();
                let views = mount_views.clone();
                let services = mount_services.clone();
                Box::pin(async move {
                    let mut context =
                        context.ok_or_else(|| MountError("test window closed".into()))?;
                    let page = PageContext {
                        tab_id: entry
                            .tab_id()
                            .map_err(|error| MountError(error.to_string()))?,
                        entry,
                        navigator,
                        services,
                    };
                    context
                        .update(|window, cx| {
                            views.mount_prepared(&destination, prepared, page, window, cx)
                        })
                        .map_err(|error| MountError(error.to_string()))?
                        .map(|view| Rc::new(view) as Resource)
                })
            });
        let router = Router::with_prepared_mount(
            RouterConfig::default(),
            services.diagnostics(),
            prepare,
            mount,
        );
        router.bind_initial_window(WindowId(1), contexts);
        router.client().set_services(services.registry());
        let driven = router.clone();
        let driver = cx.spawn(async move |_, _| driven.run().await.unwrap());
        Host {
            router,
            _driver: driver,
        }
    })
}

fn router_for(cx: &gpui::TestAppContext, window: gpui::WindowHandle<Host>) -> Router<Destination> {
    window.read_with(cx, |host, _| host.router.clone()).unwrap()
}

fn stop(
    cx: &mut gpui::TestAppContext,
    window: gpui::WindowHandle<Host>,
    router: Router<Destination>,
) {
    router.shutdown();
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    drop(router);
}

#[test]
fn loaded_routes_prepare_typed_non_clone_data_before_mount_and_retain_on_back() {
    let mut cx = gpui::TestAppContext::single();
    let (services, probe) = services(Probe::default());
    let window = add_host(&mut cx, services);
    let router = router_for(&cx, window);
    let operation = router
        .navigator()
        .open(Destination::Home(42), OpenTarget::NewTab);
    cx.run_until_parked();
    assert!(matches!(
        operation.wait().now_or_never().unwrap(),
        Ok(Outcome::Committed(_))
    ));
    assert_eq!(probe.loads.get(), 1);
    assert_eq!(probe.mounts.get(), 1);
    assert_eq!(probe.loaded_ids.borrow().as_slice(), &[42]);
    assert_eq!(probe.mounted_ids.borrow().as_slice(), &[42]);
    let tab = router.navigator().tab(router.snapshot().selected.unwrap());
    let other = tab.navigate(Destination::Other);
    cx.run_until_parked();
    assert!(matches!(
        other.wait().now_or_never().unwrap(),
        Ok(Outcome::Committed(_))
    ));
    let back = tab.back();
    cx.run_until_parked();
    assert!(matches!(
        back.wait().now_or_never().unwrap(),
        Ok(Outcome::Committed(_))
    ));
    assert_eq!(probe.loads.get(), 1, "back reuses retained loaded page");
    assert_eq!(probe.mounts.get(), 1, "back reuses retained entity");
    stop(&mut cx, window, router);
}

#[test]
fn loaded_route_cancellation_drops_before_entity_mount() {
    let mut cx = gpui::TestAppContext::single();
    let probe = Probe::default();
    let (release, receiver) = oneshot::channel();
    *probe.pending_gate.borrow_mut() = Some(receiver);
    let (services, probe) = services(probe);
    let window = add_host(&mut cx, services);
    let router = router_for(&cx, window);
    let operation = router
        .navigator()
        .open(Destination::Home(13), OpenTarget::NewTab);
    cx.run_until_parked();
    assert_eq!(router.snapshot().tabs.len(), 0);
    router.cancel(operation.id()).unwrap();
    cx.run_until_parked();
    assert!(matches!(
        operation.wait().now_or_never().unwrap(),
        Ok(Outcome::Cancelled(_))
    ));
    assert_eq!(probe.mounts.get(), 0, "cancelled preparation never mounts");
    let _ = release.send(());
    stop(&mut cx, window, router);
}

#[test]
fn loaded_route_failure_preserves_old_entry_and_retry_reloads_then_mounts() {
    let mut cx = gpui::TestAppContext::single();
    let initial_probe = Probe::default();
    initial_probe.fail_once.set(true);
    let (services, probe) = services(initial_probe);
    let window = add_host(&mut cx, services);
    let router = router_for(&cx, window);
    let opened = router
        .navigator()
        .open(Destination::Home(42), OpenTarget::NewTab);
    cx.run_until_parked();
    assert!(matches!(
        opened.wait().now_or_never().unwrap(),
        Ok(Outcome::Committed(_))
    ));

    let tab = router
        .navigator()
        .current_tab()
        .expect("initial route creates a current tab");
    let before = router.snapshot();
    let failed = tab.navigate(Destination::Home(99));
    cx.run_until_parked();
    assert!(matches!(
        failed.wait().now_or_never().unwrap(),
        Err(OperationError::Mount(_))
    ));
    let after = router.snapshot();
    assert_eq!(after.selected, before.selected);
    assert_eq!(after.tabs[0].cursor, before.tabs[0].cursor);
    assert_eq!(probe.mounted_ids.borrow().as_slice(), &[42]);

    let failed_status = router
        .navigation(WindowId(1))
        .into_iter()
        .find(|status| matches!(status.state, NavigationState::Failed(_)))
        .expect("failed loaded navigation is retained for retry");
    let retry = router
        .retry_navigation(&failed_status.key)
        .expect("failed load can be retried");
    cx.run_until_parked();
    assert!(matches!(
        retry.wait().now_or_never().unwrap(),
        Ok(Outcome::Committed(_))
    ));
    assert_eq!(probe.loads.get(), 3, "initial, failed and retried loads");
    assert_eq!(probe.loaded_ids.borrow().as_slice(), &[42, 99]);
    assert_eq!(probe.mounted_ids.borrow().as_slice(), &[42, 99]);
    stop(&mut cx, window, router);
}

#[test]
fn prepared_page_downcast_rejects_wrong_type() {
    let prepared = PreparedPage::from_value(7_u32);
    assert!(prepared.downcast::<String>().is_err());
}
