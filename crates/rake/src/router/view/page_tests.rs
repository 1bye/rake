//! Covers: registered page types, typed parameters, retention and failed-mount rollback.
//! Real: public registration, services, router driver and GPUI entity lifetimes.
//! Doubles: GPUI TestPlatform; native window observation is omitted (no OS handle).
//! Requires: view-test-support; no display or network.
#![cfg(all(feature = "view-test-support", not(target_family = "wasm")))]

use super::PageContext;
use crate::gpui::{self, Context, Entity, Task, WeakEntity, Window, prelude::*};
use crate::prelude::*;
use crate::router::runtime::{Mount, Resource};
use crate::router::{MountError, OperationError};
use futures::FutureExt as _;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone, crate::Route)]
enum Destination {
    #[route(id = "home", path = "/")]
    Home,
    #[route(id = "document", path = "/documents/{id}")]
    Document(u64),
}

#[derive(Default)]
struct Probe {
    home: RefCell<Option<WeakEntity<Home>>>,
    mounts: Cell<usize>,
    document: Cell<Option<u64>>,
}

struct Home {
    edits: usize,
}

impl Page<Destination> for Home {
    fn mount(
        _: &Destination,
        page: PageContext<Destination>,
        _: &mut Window,
        cx: &mut gpui::App,
    ) -> Result<Entity<Self>, MountError> {
        let probe = page
            .services()
            .get::<Probe>()
            .map_err(|e| MountError(e.to_string()))?;
        probe.mounts.set(probe.mounts.get() + 1);
        let view = cx.new(|_| Self { edits: 0 });
        *probe.home.borrow_mut() = Some(view.downgrade());
        Ok(view)
    }
}

impl Render for Home {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::div().child(self.edits.to_string())
    }
}

struct Document;

impl Page<Destination> for Document {
    fn mount(
        destination: &Destination,
        page: PageContext<Destination>,
        _: &mut Window,
        cx: &mut gpui::App,
    ) -> Result<Entity<Self>, MountError> {
        let Destination::Document(id) = destination else {
            return Err(MountError("document destination required".into()));
        };
        if *id == 99 {
            return Err(MountError("document unavailable".into()));
        }
        let probe = page
            .services()
            .get::<Probe>()
            .map_err(|e| MountError(e.to_string()))?;
        probe.document.set(Some(*id));
        Ok(cx.new(|_| Self))
    }
}

impl Render for Document {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::div()
    }
}

struct Host {
    router: Router<Destination>,
    _driver: Task<()>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        RouteOutlet::new(self.router.clone())
    }
}

#[test]
fn pages_share_services_retain_entities_and_roll_back_mount_errors() {
    let mut cx = gpui::TestAppContext::single();
    let registry = ServiceRegistry::new().with(Probe::default()).unwrap();
    let probe = registry.get::<Probe>().unwrap();
    let services = Services::new(
        HttpClient::new(HttpConfig::new()).unwrap(),
        Collector::default(),
        registry,
    );
    let window = cx.add_window(|window, cx| {
        let views = crate::routes! {
            Destination::Home => Home,
            Destination::Document(..) => Document,
        };
        // TestPlatform windows cannot expose a native NSView handle. Exercise the
        // existing router core and real page entities without native host observation.
        let context = window.to_async(cx);
        let views = Rc::new(views);
        let collector = services.diagnostics();
        let mount: Mount<Destination> = Rc::new(move |destination, entry, navigator| {
            let page = PageContext {
                tab_id: entry.tab_id().expect("test entry tab"),
                entry,
                navigator,
                services: services.clone(),
            };
            let mut context = context.clone();
            let views = Rc::clone(&views);
            Box::pin(async move {
                context
                    .update(|window, cx| views.mount(&destination, page, window, cx))
                    .map_err(|error| MountError(error.to_string()))?
                    .map(|view| Rc::new(view) as Resource)
            })
        });
        let router = Router::with_mount(RouterConfig::default(), collector, mount);
        let driven = router.clone();
        let driver = cx.spawn(async move |_, _| driven.run().await.expect("router driver"));
        Host {
            router,
            _driver: driver,
        }
    });
    let router = window
        .read_with(&cx, |host, _| host.router.clone())
        .unwrap();
    let open = router
        .navigator()
        .open(Destination::Home, OpenTarget::NewTab);
    cx.run_until_parked();
    assert!(matches!(
        open.wait().now_or_never().unwrap(),
        Ok(Outcome::Committed(_))
    ));
    let home = probe.home.borrow().as_ref().unwrap().upgrade().unwrap();
    home.update(&mut cx, |home, _| home.edits = 17);
    let tab = router.navigator().tab(router.snapshot().selected.unwrap());

    let visit = tab.navigate(Destination::Document(42));
    cx.run_until_parked();
    assert!(matches!(
        visit.wait().now_or_never().unwrap(),
        Ok(Outcome::Committed(_))
    ));
    assert_eq!(probe.document.get(), Some(42));
    let back = tab.back();
    cx.run_until_parked();
    assert!(matches!(
        back.wait().now_or_never().unwrap(),
        Ok(Outcome::Committed(_))
    ));
    assert_eq!(probe.mounts.get(), 1, "Back must reuse the retained page");
    assert_eq!(cx.read(|cx| home.read(cx).edits), 17);

    let before = router.snapshot();
    let failed = tab.navigate(Destination::Document(99));
    cx.run_until_parked();
    assert!(matches!(
        failed.wait().now_or_never().unwrap(),
        Err(OperationError::Mount(_))
    ));
    let after = router.snapshot();
    assert_eq!(before.tabs[0].cursor, after.tabs[0].cursor);
    assert_eq!(before.tabs[0].entries.len(), after.tabs[0].entries.len());
    assert_eq!(
        cx.read(|cx| home.read(cx).edits),
        17,
        "failed mounting preserves the current page"
    );
    router.shutdown();
    cx.run_until_parked();
}
