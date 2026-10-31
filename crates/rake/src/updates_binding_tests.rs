//! GPUI-facing tests for the shared update binding.
//!
//! These use a disabled controller because it is the only provider-neutral
//! controller state that can be constructed without network, native SDKs or
//! private controller test hooks. Guarded-restart approval/decline remains
//! covered by the controller/router suites; this module checks the binding
//! boundary without introducing a shipping test backend.
#![cfg(all(feature = "view-test-support", not(target_family = "wasm")))]

use super::*;
use crate::diagnostics::Collector;
use crate::gpui::{self, Context, Entity, Render, Task, WeakEntity, Window, prelude::*};
use crate::http::{HttpClient, HttpConfig};
use crate::prelude::Runtime;
use crate::router::runtime::{Mount, Resource};
use crate::router::{
    MountError, NoRoute, OpenTarget, Page, PageContext, RouteOutlet, RouteViews, Router,
    RouterConfig,
};
use crate::services::{ServiceRegistry, Services};
use futures::FutureExt as _;
use rake_updates::{UpdateController, UpdateService};
use std::cell::{Cell, RefCell};
use std::fs;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

fn disabled_controller() -> UpdateController {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "rake-updates-binding-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();

    let policy = r#"{"schema_version":1,"identifier":"dev.rake.updates-binding","version":"1.0.0","package_version":"1.0.0","target":"x86_64-unknown-linux-gnu","format":"deb","owner":"disabled","channel":"stable","base_url":null,"public_keys":{}}"#;
    let installation = serde_json::json!({
        "schema_version": 1,
        "identifier": "dev.rake.updates-binding",
        "version": "1.0.0",
        "package_version": "1.0.0",
        "target": "x86_64-unknown-linux-gnu",
        "platform": "linux",
        "format": "deb",
        "update_owner": "disabled",
        "updates": {},
        "helpers": {},
        "runtimes": {},
        "frameworks": {},
        "native_groups": {}
    });
    fs::write(
        root.join("rake-installation.json"),
        serde_json::to_vec(&installation).unwrap(),
    )
    .unwrap();
    let resources = rake_resources::Resources::load(&root).unwrap();
    let service = UpdateService::new(policy, &resources).unwrap();
    fs::remove_dir_all(root).unwrap();
    UpdateController::new(service)
}

fn services(controller: Option<UpdateController>) -> Services {
    let registry = controller.map_or_else(ServiceRegistry::new, |controller| {
        ServiceRegistry::new().with(controller).unwrap()
    });
    Services::new(
        HttpClient::new(HttpConfig::new()).unwrap(),
        Collector::default(),
        registry,
    )
}

struct Harness {
    updates: Option<UpdateHandle>,
    router: Router<NoRoute>,
    _driver: Task<()>,
    renders: Rc<Cell<usize>>,
}

impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        gpui::div()
    }
}

fn harness(
    cx: &mut gpui::TestAppContext,
    controller: Option<UpdateController>,
    renders: Rc<Cell<usize>>,
) -> gpui::WindowHandle<Harness> {
    cx.add_window(move |_, cx| {
        let runtime = Runtime::new(RouterConfig::default(), services(controller));
        let context = runtime.context();
        let router = runtime.router.clone();
        let driver = cx.spawn(async move |_, _| {
            let _ = runtime.run().await;
        });
        let updates = context.bind_updates(cx);
        Harness {
            updates: Some(updates),
            router,
            _driver: driver,
            renders,
        }
    })
}

struct PairHarness {
    first: UpdateHandle,
    second: UpdateHandle,
    router: Router<NoRoute>,
    _driver: Task<()>,
}

impl Render for PairHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::div()
    }
}

fn pair_harness(
    cx: &mut gpui::TestAppContext,
    controller: UpdateController,
) -> gpui::WindowHandle<PairHarness> {
    cx.add_window(move |_, cx| {
        let runtime = Runtime::new(RouterConfig::default(), services(Some(controller)));
        let context = runtime.context();
        let router = runtime.router.clone();
        let driver = cx.spawn(async move |_, _| {
            let _ = runtime.run().await;
        });
        let first = context.bind_updates(cx);
        let second = context.bind_updates(cx);
        PairHarness {
            first,
            second,
            router,
            _driver: driver,
        }
    })
}

#[derive(Clone, crate::Route)]
enum PageDestination {
    #[route(id = "updates-home", path = "/updates-home")]
    Home,
    #[route(id = "updates-other", path = "/updates-other")]
    Other,
}

#[derive(Default)]
struct PageProbe {
    home: RefCell<Option<WeakEntity<HomePage>>>,
    controller: RefCell<Option<Rc<UpdateController>>>,
}

struct HomePage {
    updates: UpdateHandle,
}

impl Page<PageDestination> for HomePage {
    fn mount(
        page_destination: &PageDestination,
        page: PageContext<PageDestination>,
        _: &mut Window,
        cx: &mut gpui::App,
    ) -> Result<Entity<Self>, MountError> {
        if !matches!(page_destination, PageDestination::Home) {
            return Err(MountError("updates home destination required".into()));
        }
        let probe = page
            .services()
            .get::<PageProbe>()
            .map_err(|error| MountError(error.to_string()))?;
        let view = cx.new(|cx| Self {
            updates: page.bind_updates(cx),
        });
        *probe.home.borrow_mut() = Some(view.downgrade());
        Ok(view)
    }
}

impl Render for HomePage {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::div()
    }
}

struct OtherPage;

impl Page<PageDestination> for OtherPage {
    fn mount(
        page_destination: &PageDestination,
        _: PageContext<PageDestination>,
        _: &mut Window,
        cx: &mut gpui::App,
    ) -> Result<Entity<Self>, MountError> {
        if !matches!(page_destination, PageDestination::Other) {
            return Err(MountError("updates other destination required".into()));
        }
        Ok(cx.new(|_| Self))
    }
}

impl Render for OtherPage {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::div()
    }
}

struct PageHarness {
    router: Router<PageDestination>,
    _driver: Task<()>,
}

impl Render for PageHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        RouteOutlet::new(self.router.clone())
    }
}

fn page_harness(
    cx: &mut gpui::TestAppContext,
    controller: UpdateController,
    config: RouterConfig,
) -> (gpui::WindowHandle<PageHarness>, Rc<PageProbe>) {
    let registry = ServiceRegistry::new()
        .with(PageProbe::default())
        .unwrap()
        .with(controller)
        .unwrap();
    let services = Services::new(
        HttpClient::new(HttpConfig::new()).unwrap(),
        Collector::default(),
        registry,
    );
    let probe = services.get::<PageProbe>().unwrap();
    *probe.controller.borrow_mut() = Some(services.get::<UpdateController>().unwrap());
    let window = cx.add_window(move |window, cx| {
        let views = Rc::new(crate::routes! {
            PageDestination::Home => HomePage,
            PageDestination::Other => OtherPage,
        });
        let context = window.to_async(cx);
        let collector = services.diagnostics();
        let registry = services.registry();
        let mount: Mount<PageDestination> = Rc::new(move |destination, entry, navigator| {
            let page =
                PageContext::new(navigator, entry, services.clone()).expect("test entry tab");
            let mut context = context.clone();
            let views = Rc::clone(&views);
            Box::pin(async move {
                context
                    .update(|window, cx| views.mount(&destination, page, window, cx))
                    .map_err(|error| MountError(error.to_string()))?
                    .map(|view| Rc::new(view) as Resource)
            })
        });
        let router = Router::with_mount(config, collector, mount);
        router.client().set_services(registry);
        let driven = router.clone();
        let driver = cx.spawn(async move |_, _| {
            let _ = driven.run().await;
        });
        PageHarness {
            router,
            _driver: driver,
        }
    });
    (window, probe)
}

fn draw(cx: &mut gpui::TestAppContext, window: gpui::WindowHandle<impl Render>) {
    cx.update_window(window.into(), |_, window, cx| {
        window.draw(cx).clear(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[test]
fn absent_policy_is_unavailable_and_actions_retain_errors() {
    let mut cx = gpui::TestAppContext::single();
    let renders = Rc::new(Cell::new(0));
    let window = harness(&mut cx, None, renders.clone());
    draw(&mut cx, window);

    window
        .read_with(&cx, |host, cx| {
            let updates = host.updates.as_ref().unwrap();
            assert_eq!(updates.state(cx), UpdateHandleState::Unavailable);
            assert!(updates.available_version(cx).is_none());
            assert!(!updates.can_check(cx));
            assert!(!updates.can_download(cx));
            assert!(!updates.can_restart(cx));
            assert!(!updates.can_cancel(cx));
        })
        .unwrap();

    let before = renders.get();
    window
        .update(&mut cx, |host, _, cx| {
            host.updates.as_ref().unwrap().check(cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert!(
        renders.get() > before,
        "action error should redraw the owner"
    );
    window
        .read_with(&cx, |host, cx| {
            assert!(
                host.updates
                    .as_ref()
                    .unwrap()
                    .error(cx)
                    .unwrap()
                    .contains("update binding is unavailable")
            );
        })
        .unwrap();

    window
        .read_with(&cx, |host, _| host.router.clone())
        .unwrap()
        .shutdown();
    cx.run_until_parked();
}

#[test]
fn disabled_controller_rejects_actions_and_provider_failures_are_readable() {
    let mut cx = gpui::TestAppContext::single();
    let window = harness(&mut cx, Some(disabled_controller()), Rc::new(Cell::new(0)));
    draw(&mut cx, window);

    window
        .update(&mut cx, |host, _, cx| {
            let updates = host.updates.as_ref().unwrap();
            updates.check(cx);
            updates.download(cx);
            updates.restart(cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .read_with(&cx, |host, cx| {
            let updates = host.updates.as_ref().unwrap();
            assert!(matches!(
                updates.state(cx),
                UpdateHandleState::Observed(UpdateState::Disabled)
            ));
            assert!(updates.error(cx).is_some());
        })
        .unwrap();

    window
        .update(&mut cx, |host, _, cx| {
            let updates = host.updates.as_ref().unwrap();
            updates.model.update(cx, |model, model_cx| {
                model.snapshot = Some(UpdateSnapshot {
                    revision: model
                        .snapshot
                        .as_ref()
                        .map_or(1, |snapshot| snapshot.revision + 1),
                    state: UpdateState::Failed {
                        message: "provider fixture failure".into(),
                    },
                });
                model.last_error = None;
                model_cx.notify();
            });
        })
        .unwrap();
    window
        .read_with(&cx, |host, cx| {
            assert_eq!(
                host.updates.as_ref().unwrap().error(cx).as_deref(),
                Some("provider fixture failure")
            );
        })
        .unwrap();

    window
        .read_with(&cx, |host, _| host.router.clone())
        .unwrap()
        .shutdown();
    cx.run_until_parked();
}

#[test]
fn handles_retain_the_same_shared_controller_identity() {
    let mut cx = gpui::TestAppContext::single();
    let window = pair_harness(&mut cx, disabled_controller());
    draw(&mut cx, window);
    window
        .read_with(&cx, |host, cx| {
            let first = host
                .first
                .model
                .read(cx)
                .controller
                .as_ref()
                .unwrap()
                .clone();
            let second = host
                .second
                .model
                .read(cx)
                .controller
                .as_ref()
                .unwrap()
                .clone();
            assert!(Rc::ptr_eq(&first, &second));
        })
        .unwrap();
    window
        .read_with(&cx, |host, _| host.router.clone())
        .unwrap()
        .shutdown();
    cx.run_until_parked();
}

#[test]
fn controller_changes_before_the_first_poll_are_not_missed() {
    let mut cx = gpui::TestAppContext::single();
    let window = harness(&mut cx, Some(disabled_controller()), Rc::new(Cell::new(0)));

    // The GPUI task has been admitted but has not been driven yet. Mutating the
    // shared controller now verifies that the stored initial revision is used.
    window
        .update(&mut cx, |host, _, cx| {
            host.updates
                .as_ref()
                .unwrap()
                .model
                .read(&*cx)
                .controller
                .as_ref()
                .unwrap()
                .cancel();
        })
        .unwrap();
    cx.run_until_parked();
    window
        .read_with(&cx, |host, cx| {
            assert!(
                host.updates
                    .as_ref()
                    .unwrap()
                    .model
                    .read(cx)
                    .snapshot
                    .as_ref()
                    .unwrap()
                    .revision
                    > 0
            );
        })
        .unwrap();
    window
        .read_with(&cx, |host, _| host.router.clone())
        .unwrap()
        .shutdown();
    cx.run_until_parked();
}

#[test]
fn hidden_page_refreshes_the_shared_snapshot_when_visible_again() {
    let mut cx = gpui::TestAppContext::single();
    let (window, probe) = page_harness(&mut cx, disabled_controller(), RouterConfig::default());
    let router = window
        .read_with(&cx, |host, _| host.router.clone())
        .unwrap();
    let open = router
        .navigator()
        .open(PageDestination::Home, OpenTarget::NewTab);
    cx.run_until_parked();
    assert!(open.wait().now_or_never().is_some());
    let home = probe.home.borrow().as_ref().unwrap().upgrade().unwrap();
    let tab = router.navigator().tab(router.snapshot().selected.unwrap());
    let before = cx.read(|cx| {
        home.read(cx)
            .updates
            .model
            .read(cx)
            .snapshot
            .as_ref()
            .unwrap()
            .revision
    });

    let moved = tab.navigate(PageDestination::Other);
    cx.run_until_parked();
    assert!(moved.wait().now_or_never().is_some());
    probe.controller.borrow().as_ref().unwrap().cancel();
    cx.run_until_parked();

    let returned = tab.back();
    cx.run_until_parked();
    assert!(returned.wait().now_or_never().is_some());
    let after = cx.read(|cx| {
        home.read(cx)
            .updates
            .model
            .read(cx)
            .snapshot
            .as_ref()
            .unwrap()
            .revision
    });
    assert!(
        after > before,
        "visible restart should refresh a hidden page"
    );
    router.shutdown();
    cx.run_until_parked();
}

#[test]
fn owner_release_clears_a_moved_handle() {
    let mut cx = gpui::TestAppContext::single();
    let window = harness(&mut cx, Some(disabled_controller()), Rc::new(Cell::new(0)));
    draw(&mut cx, window);
    let retained = Rc::new(RefCell::new(None));
    let retained_in_window = retained.clone();
    window
        .update(&mut cx, |host, _, _| {
            *retained_in_window.borrow_mut() = host.updates.take();
        })
        .unwrap();
    window
        .update(&mut cx, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    cx.read(|cx| {
        assert_eq!(
            retained.borrow().as_ref().unwrap().state(cx),
            UpdateHandleState::Unavailable
        );
    });
}

#[test]
fn disposed_entry_disables_a_still_retained_page_binding() {
    let mut cx = gpui::TestAppContext::single();
    let (window, probe) = page_harness(&mut cx, disabled_controller(), RouterConfig::default());
    let router = window
        .read_with(&cx, |host, _| host.router.clone())
        .unwrap();
    let open = router
        .navigator()
        .open(PageDestination::Home, OpenTarget::NewTab);
    cx.run_until_parked();
    assert!(open.wait().now_or_never().is_some());
    let home = probe.home.borrow().as_ref().unwrap().upgrade().unwrap();
    let tab = router.navigator().current_tab().unwrap();
    let close = tab.close();
    cx.run_until_parked();
    assert!(close.wait().now_or_never().is_some());
    cx.read(|cx| {
        assert_eq!(
            home.read(cx).updates.state(cx),
            UpdateHandleState::Unavailable
        );
        assert!(!home.read(cx).updates.can_check(cx));
    });
    router.shutdown();
    cx.run_until_parked();
}

#[test]
fn failed_observer_admission_is_unavailable_with_a_visible_error() {
    let mut cx = gpui::TestAppContext::single();
    let mut config = RouterConfig::default();
    config.tasks.capacity = 0;
    let (window, probe) = page_harness(&mut cx, disabled_controller(), config);
    let router = window
        .read_with(&cx, |host, _| host.router.clone())
        .unwrap();
    let open = router
        .navigator()
        .open(PageDestination::Home, OpenTarget::NewTab);
    cx.run_until_parked();
    assert!(open.wait().now_or_never().is_some());
    let home = probe.home.borrow().as_ref().unwrap().upgrade().unwrap();
    cx.read(|cx| {
        let updates = &home.read(cx).updates;
        assert_eq!(updates.state(cx), UpdateHandleState::Unavailable);
        assert!(updates.error(cx).is_some());
        assert!(!updates.can_check(cx));
    });
    router.shutdown();
    cx.run_until_parked();
}

#[test]
fn runtime_shutdown_marks_a_live_handle_unavailable() {
    let mut cx = gpui::TestAppContext::single();
    let window = harness(&mut cx, Some(disabled_controller()), Rc::new(Cell::new(0)));
    draw(&mut cx, window);
    let router = window
        .read_with(&cx, |host, _| host.router.clone())
        .unwrap();
    assert!(
        router
            .snapshot()
            .work
            .tasks
            .iter()
            .all(|task| !task.key.starts_with("rake.updates."))
    );
    let quit = router.quit(crate::quit::QuitOptions::default());
    cx.run_until_parked();
    assert_eq!(router.quit_status().phase, crate::quit::QuitPhase::Ready);
    drop(quit);
    window
        .read_with(&cx, |host, cx| {
            let updates = host.updates.as_ref().unwrap();
            assert_eq!(updates.state(cx), UpdateHandleState::Unavailable);
            assert!(updates.error(cx).is_some());
        })
        .unwrap();
}
