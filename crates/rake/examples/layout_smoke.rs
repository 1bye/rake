//! Native smoke fixture for persistent layouts and failed secondary windows.
//!
//! This intentionally uses the application's workspace driver. It must not create a
//! second `Runtime` or call `Router::run`; doing either would hide the public app API
//! contract this fixture is meant to exercise.

#![cfg_attr(
    any(not(feature = "gpui"), target_family = "wasm"),
    allow(dead_code, unused_imports)
)]

#[cfg(all(feature = "gpui", not(target_family = "wasm")))]
mod native {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use rake::gpui::{self, Context, Entity, Task, Window, WindowOptions, prelude::*};
    use rake::prelude::*;
    use rake::router::{MountError, Operation, OperationError, WindowId};

    const INITIAL_WINDOW: WindowId = WindowId(1);

    #[derive(Clone, rake::Route)]
    enum Destination {
        #[route(id = "home", path = "/")]
        Home,
        #[route(id = "other", path = "/other")]
        Other,
    }

    /// All fields are shared so the value registered in `Services` and the copy
    /// captured by the application factory observe the same probe state.
    #[derive(Clone)]
    struct ProbeService {
        pool: Rc<()>,
        layout_calls: Rc<Cell<usize>>,
        initial_layout_calls: Rc<Cell<usize>>,
        failed_layout_calls: Rc<Cell<usize>>,
        first_http: Rc<RefCell<Option<HttpClient>>>,
        page_mounts: Rc<Cell<usize>>,
        page_pool_checks: Rc<Cell<usize>>,
        other_mount_windows: Rc<RefCell<Vec<WindowId>>>,
        options_calls: Rc<Cell<usize>>,
        option_ids: Rc<RefCell<Vec<WindowId>>>,
        failed_layout_window: Rc<Cell<Option<WindowId>>>,
        initial_revision: Rc<Cell<Option<u64>>>,
        shell_entity: Rc<Cell<Option<gpui::EntityId>>>,
    }

    impl Default for ProbeService {
        fn default() -> Self {
            Self {
                pool: Rc::new(()),
                layout_calls: Rc::new(Cell::new(0)),
                initial_layout_calls: Rc::new(Cell::new(0)),
                failed_layout_calls: Rc::new(Cell::new(0)),
                first_http: Rc::new(RefCell::new(None)),
                page_mounts: Rc::new(Cell::new(0)),
                page_pool_checks: Rc::new(Cell::new(0)),
                other_mount_windows: Rc::new(RefCell::new(Vec::new())),
                options_calls: Rc::new(Cell::new(0)),
                option_ids: Rc::new(RefCell::new(Vec::new())),
                failed_layout_window: Rc::new(Cell::new(None)),
                initial_revision: Rc::new(Cell::new(None)),
                shell_entity: Rc::new(Cell::new(None)),
            }
        }
    }

    struct Home;
    struct Other;

    fn page_probe<R: Route>(page: &PageContext<R>) -> Result<Rc<ProbeService>, MountError> {
        let service = page
            .services()
            .get::<ProbeService>()
            .map_err(|error| MountError(error.to_string()))?;
        let same = page
            .services()
            .get::<ProbeService>()
            .map_err(|error| MountError(error.to_string()))?;
        assert!(
            Rc::ptr_eq(&service, &same),
            "pages must use one service pool"
        );
        assert!(
            Rc::ptr_eq(&service.pool, &same.pool),
            "service pool identity changed"
        );
        service
            .first_http
            .borrow()
            .as_ref()
            .map(|http| {
                assert!(
                    page.services().http().shares_transport_with(http),
                    "page standard HTTP must share the first layout transport"
                );
            })
            .ok_or_else(|| MountError("layout did not capture standard HTTP".into()))?;
        service
            .page_pool_checks
            .set(service.page_pool_checks.get() + 1);
        Ok(service)
    }

    impl Page<Destination> for Home {
        fn mount(
            _: &Destination,
            page: PageContext<Destination>,
            _: &mut Window,
            cx: &mut gpui::App,
        ) -> Result<Entity<Self>, MountError> {
            let service = page_probe(&page)?;
            service.page_mounts.set(service.page_mounts.get() + 1);
            Ok(cx.new(|_| Self))
        }
    }

    impl Render for Home {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            gpui::div().child("home")
        }
    }

    impl Page<Destination> for Other {
        fn mount(
            _: &Destination,
            page: PageContext<Destination>,
            _: &mut Window,
            cx: &mut gpui::App,
        ) -> Result<Entity<Self>, MountError> {
            let service = page_probe(&page)?;
            service
                .other_mount_windows
                .borrow_mut()
                .push(page.window_id());
            service.page_mounts.set(service.page_mounts.get() + 1);
            Ok(cx.new(|_| Self))
        }
    }

    impl Render for Other {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            gpui::div().child("other")
        }
    }

    struct Shell {
        layout: LayoutContext<Destination>,
        local_counter: usize,
        workflow: Option<Task<()>>,
    }

    impl Shell {
        fn mount(
            layout: LayoutContext<Destination>,
            _: &mut Window,
            cx: &mut gpui::App,
        ) -> Result<Entity<Self>, MountError> {
            let service = layout
                .services()
                .get::<ProbeService>()
                .map_err(|error| MountError(error.to_string()))?;
            let same = layout
                .services()
                .get::<ProbeService>()
                .map_err(|error| MountError(error.to_string()))?;
            assert!(
                Rc::ptr_eq(&service, &same),
                "layout must use one service pool"
            );

            let router = layout
                .router()
                .map_err(|error| MountError(error.to_string()))?;
            service.layout_calls.set(service.layout_calls.get() + 1);
            if layout.window_id() == INITIAL_WINDOW {
                service
                    .initial_layout_calls
                    .set(service.initial_layout_calls.get() + 1);
                *service.first_http.borrow_mut() = Some(layout.services().http());
                service
                    .initial_revision
                    .set(Some(router.snapshot().revision));
            }

            let entity = cx.new(|_| Self {
                layout: layout.clone(),
                local_counter: 0,
                workflow: None,
            });
            if layout.window_id() == INITIAL_WINDOW {
                service.shell_entity.set(Some(entity.entity_id()));
                let weak = entity.downgrade();
                let task = cx.spawn(async move |cx| run_workflow(weak, router, service, cx).await);
                entity.update(cx, |shell, _| shell.workflow = Some(task));
            }
            Ok(entity)
        }
    }

    impl Render for Shell {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            gpui::div()
                .size_full()
                .child(format!("layout smoke {}", self.local_counter))
                .child(self.layout.outlet())
        }
    }

    fn current_route(router: &Router<Destination>) -> Option<String> {
        let snapshot = router.snapshot_in_window(INITIAL_WINDOW).ok()?;
        let selected = snapshot.selected?;
        let tab = snapshot.tabs.iter().find(|tab| tab.id == selected)?;
        tab.entries
            .get(tab.cursor)
            .map(|entry| entry.destination.route.clone())
    }

    async fn committed<T: Clone>(operation: Operation<T>) {
        assert!(
            matches!(operation.wait().await, Ok(Outcome::Committed(_))),
            "operation did not commit"
        );
    }

    async fn wait_for_route(router: &Router<Destination>, route: &str, mut revision: u64) {
        loop {
            if current_route(router).as_deref() == Some(route) {
                return;
            }
            revision = router.changed(revision).await;
        }
    }

    async fn run_workflow(
        view: gpui::WeakEntity<Shell>,
        router: Router<Destination>,
        service: Rc<ProbeService>,
        cx: &mut gpui::AsyncApp,
    ) {
        let initial_revision = service
            .initial_revision
            .get()
            .expect("initial layout must record a revision");
        let revision = router.changed(initial_revision).await;
        wait_for_route(&router, "home", revision).await;

        assert_eq!(service.initial_layout_calls.get(), 1);
        assert_eq!(service.layout_calls.get(), 1);
        assert!(service.first_http.borrow().is_some());
        assert_eq!(service.shell_entity.get(), Some(view.entity_id()));

        let tab_id = router
            .snapshot_in_window(INITIAL_WINDOW)
            .expect("initial window exists")
            .selected
            .expect("initial tab exists");
        let tab = router.navigator().for_window(INITIAL_WINDOW).tab(tab_id);
        let before_navigation = router.snapshot().revision;
        committed(tab.navigate(Destination::Other)).await;
        let revision = router.changed(before_navigation).await;
        wait_for_route(&router, "other", revision).await;

        view.update(cx, |shell, _| shell.local_counter += 1)
            .expect("shell survives navigation");
        let before_back = router.snapshot().revision;
        committed(tab.back()).await;
        let revision = router.changed(before_back).await;
        wait_for_route(&router, "home", revision).await;
        assert_eq!(
            view.read_with(cx, |shell, _| shell.local_counter)
                .expect("shell remains readable"),
            1
        );
        assert_eq!(service.shell_entity.get(), Some(view.entity_id()));
        println!("LAYOUT_SMOKE_PASS persistent-layout");

        let open = router
            .navigator()
            .for_window(INITIAL_WINDOW)
            .open(Destination::Other, OpenTarget::NewWindow);
        let result = open.wait().await;
        assert!(
            matches!(result, Err(OperationError::Mount(_))),
            "{result:?}"
        );

        let snapshot = router.snapshot();
        assert_eq!(snapshot.windows.len(), 1, "failed window must roll back");
        assert_eq!(service.layout_calls.get(), 2);
        assert_eq!(service.initial_layout_calls.get(), 1);
        assert_eq!(service.failed_layout_calls.get(), 1);
        assert_eq!(service.options_calls.get(), 1);
        assert_eq!(
            service.option_ids.borrow().as_slice(),
            &[service
                .failed_layout_window
                .get()
                .expect("failed window id")]
        );
        assert!(
            service
                .other_mount_windows
                .borrow()
                .iter()
                .all(|window| *window == INITIAL_WINDOW)
        );
        assert_eq!(service.other_mount_windows.borrow().len(), 1);

        let restored = router
            .snapshot_in_window(INITIAL_WINDOW)
            .expect("initial window remains");
        let selected = restored.selected.expect("history remains selected");
        let tab = restored
            .tabs
            .iter()
            .find(|tab| tab.id == selected)
            .expect("history tab remains");
        assert_eq!(tab.entries.len(), 2);
        assert_eq!(tab.cursor, 0);
        assert_eq!(tab.entries[tab.cursor].destination.route, "home");
        assert!(service.page_pool_checks.get() >= 2);
        println!("LAYOUT_SMOKE_PASS failed-secondary-window");
        println!("LAYOUT_SMOKE_PASS guarded-quit-requested");
        drop(router.navigator().quit(QuitOptions::default()));
    }

    fn application() -> Result<App<Destination>, rake::ServiceError> {
        let probe = ProbeService::default();
        let layout_probe = probe.clone();
        let options_probe = probe.clone();

        Ok(App::new("example.layout-smoke")
            .service(probe)?
            .routes(rake::routes! {
                Destination::Home => Home,
                Destination::Other => Other,
            })
            .try_layout(move |layout, window, cx| {
                if layout.window_id() != INITIAL_WINDOW {
                    layout_probe
                        .layout_calls
                        .set(layout_probe.layout_calls.get() + 1);
                    layout_probe
                        .failed_layout_calls
                        .set(layout_probe.failed_layout_calls.get() + 1);
                    layout_probe
                        .failed_layout_window
                        .set(Some(layout.window_id()));
                    return Err(MountError("intentional secondary layout failure".into()));
                }
                // Shell records the first layout's services, revision and entity.
                Shell::mount(layout, window, cx)
            })
            .new_window_options(move |window_id| {
                options_probe
                    .options_calls
                    .set(options_probe.options_calls.get() + 1);
                options_probe.option_ids.borrow_mut().push(window_id);
                WindowOptions::default()
            })
            .initial(Destination::Home)
            .workspace_chrome(false))
    }

    pub(super) fn run() -> Result<(), rake::AppError> {
        application()?.run()
    }
}

#[cfg(all(feature = "gpui", not(target_family = "wasm")))]
fn main() -> Result<(), rake::AppError> {
    native::run()
}

#[cfg(any(not(feature = "gpui"), target_family = "wasm"))]
fn main() {
    eprintln!("the layout smoke example requires a native GPUI host");
}
