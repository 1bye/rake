//! GPUI mounting and presentation; retained entries own independent view entities.

#[cfg(all(test, feature = "view-test-support", not(target_family = "wasm")))]
#[path = "view/page_tests.rs"]
mod page_tests;

use crate::inspection::InspectElement as _;
#[cfg(not(feature = "devtools"))]
use crate::inspection::InspectRoot as _;

#[cfg(not(target_family = "wasm"))]
use futures::FutureExt as _;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{AnyView, Context, ElementId, IntoElement, Render, RenderOnce, Window, div, rgb};

use crate::Services;
use crate::tasks::{ActivityRun, AppHandle, TaskError, TaskHandle};

use super::runtime::{Guard, Mount, Resource};

use super::{
    EntryHandle, MountError, Navigator, OperationState, Route, Router, RouterConfig, TabHandle,
    TabId, WebConfig, WindowId,
};

#[cfg(not(target_family = "wasm"))]
use super::{Activation, WindowHost};

/// Application mapping from compact destinations to ordinary GPUI view instances.
pub trait RouteViews<R: Route>: 'static {
    /// Constructs a fresh screen on the UI loop, once per newly created retained entry.
    /// Domain models may be shared; local view state belongs to this entry.
    fn mount(
        &self,
        destination: &R,
        page: PageContext<R>,
        window: &mut Window,
        cx: &mut gpui::App,
    ) -> Result<AnyView, MountError>;
}

/// Owned route-scoped services and weak runtime handles, safe to capture across waits.
#[derive(Clone)]
pub struct PageContext<R: Route> {
    navigator: Navigator<R>,
    entry: EntryHandle<R>,
    services: Services,
    tab_id: TabId,
}

impl<R: Route> PageContext<R> {
    /// Application capabilities independent of this entry's navigation lifetime.
    /// The context contains weak runtime handles and preserves service identity.
    pub fn runtime(&self) -> crate::runtime::RuntimeContext<R> {
        crate::runtime::RuntimeContext::from_navigator(
            self.navigator.clone(),
            self.services.clone(),
        )
    }

    /// Dispatches a registered command through the shared operation registry.
    pub fn dispatch<C: crate::control::ApplicationCommand>(
        &self,
        command: C,
        key: String,
    ) -> Result<crate::control::Reply, crate::control::ControlError> {
        self.navigator.dispatch(command, key)
    }

    /// Application-owned work retains its causal origin and survives this tab closing.
    pub fn app(&self) -> AppHandle {
        self.navigator.app()
    }

    /// Registers fresh activity runs following the tab's Foreground/Live policy.
    pub fn activity<F, Fut>(
        &self,
        key: impl Into<String>,
        factory: F,
    ) -> Result<TaskHandle, TaskError>
    where
        F: Fn(ActivityRun) -> Fut + 'static,
        Fut: std::future::Future<Output = Result<(), TaskError>> + 'static,
    {
        self.entry.activity(key, factory)
    }

    /// Registers activity that always follows presentation and host visibility.
    pub fn while_visible<F, Fut>(
        &self,
        key: impl Into<String>,
        factory: F,
    ) -> Result<TaskHandle, TaskError>
    where
        F: Fn(ActivityRun) -> Fut + 'static,
        Fut: std::future::Future<Output = Result<(), TaskError>> + 'static,
    {
        self.entry.while_visible(key, factory)
    }

    /// Receives typed publications through the UI driver after owner/run revalidation.
    pub fn on_event<T: 'static>(
        &self,
        handler: impl Fn(T) -> Result<(), TaskError> + 'static,
    ) -> Result<(), TaskError> {
        self.entry.on_event(handler)
    }

    /// Returns a handle to this screen's original tab, independent of selection.
    pub fn tab(&self) -> TabHandle<R> {
        self.navigator.tab(self.tab_id)
    }

    /// Concrete host that owns this retained screen.
    pub fn window_id(&self) -> WindowId {
        self.navigator.window_id()
    }

    /// Returns a navigator bound to this screen's originating tab.
    pub fn navigator(&self) -> Navigator<R> {
        self.navigator.clone()
    }

    /// Returns this retained instance's dirty-state and disposal-guard handle.
    pub fn entry(&self) -> EntryHandle<R> {
        self.entry.clone()
    }

    /// Returns the shared application HTTP service.
    pub fn http(&self) -> crate::http::HttpClient {
        self.services.http()
    }

    /// Returns the standard application services.
    pub fn services(&self) -> Services {
        self.services.clone()
    }
}

impl<R: Route> Router<R> {
    /// Creates a real view-backed router in an existing UI host. The host owns `run`
    /// and observes `changed` to redraw its shell; ordinary applications use `App::routes`.
    pub fn in_window(
        config: RouterConfig,
        services: Services,
        views: impl RouteViews<R>,
        window: &Window,
        cx: &gpui::App,
    ) -> Self {
        let contexts = Rc::new(RefCell::new(BTreeMap::new()));

        Self::with_views(
            config,
            services,
            Rc::new(views),
            WindowId(1),
            contexts,
            window,
            cx,
        )
    }

    fn with_views(
        config: RouterConfig,
        services: Services,
        views: Rc<dyn RouteViews<R>>,
        initial_window: WindowId,
        contexts: Rc<RefCell<BTreeMap<WindowId, gpui::AsyncWindowContext>>>,
        window: &Window,
        cx: &gpui::App,
    ) -> Self {
        contexts
            .borrow_mut()
            .insert(initial_window, window.to_async(cx));
        let collector = services.diagnostics();
        let registry = services.registry();
        let mounting_contexts = contexts.clone();

        let mount: Mount<R> = Rc::new(move |destination, entry, navigator| {
            let context = mounting_contexts
                .borrow()
                .get(&navigator.window_id())
                .cloned();

            let views = views.clone();
            let tab_id = entry.tab_id();
            let services = services.clone();

            Box::pin(async move {
                let mut context =
                    context.ok_or_else(|| MountError("The target window has closed.".into()))?;

                let page = PageContext {
                    navigator,
                    entry,
                    services,
                    tab_id: tab_id.map_err(|error| MountError(error.to_string()))?,
                };

                context
                    .update(|window, cx| views.mount(&destination, page, window, cx))
                    .map_err(|error| MountError(error.to_string()))?
                    .map(|view| Rc::new(view) as Resource)
            })
        });

        let router = Router::with_mount(config, collector, mount);

        router.bind_initial_window(initial_window, contexts);
        router.client().set_services(registry);
        router.observe_host(window, cx);

        router
    }
}

/// A default application tab strip driven by the authoritative shared router.
#[derive(IntoElement)]
pub struct TabBar<R: Route> {
    router: Router<R>,
    window: WindowId,
}

impl<R: Route> TabBar<R> {
    /// Creates a strip. Its owning view redraws when `Router::changed` resolves.
    pub fn new(router: Router<R>) -> Self {
        let window = router.navigator().window_id();

        Self { router, window }
    }
}

impl<R: Route> TabBar<R> {
    /// Displays only tabs belonging to this concrete host.
    pub fn in_window(mut self, window: WindowId) -> Self {
        self.window = window;

        self
    }
}

impl<R: Route> RenderOnce for TabBar<R> {
    fn render(self, _window: &mut Window, _cx: &mut gpui::App) -> impl IntoElement {
        let snapshot = self.router.presented_snapshot(self.window);
        let navigator = self.router.navigator().for_window(self.window);

        div()
            .flex()
            .gap_2()
            .p_2()
            .bg(rgb(0x22343e))
            .children(snapshot.tabs.iter().map(|tab| {
                let current = &tab.entries[tab.cursor];

                let label = format!(
                    "{} {}{}",
                    current.destination.route,
                    current
                        .destination
                        .params
                        .values()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(" "),
                    if current.dirty { " •" } else { "" }
                );

                let select = navigator.clone();
                let close = navigator.clone();
                let id = tab.id;

                div()
                    .id(ElementId::NamedInteger("rake.tab".into(), id.value()))
                    .flex()
                    .gap_1()
                    .items_center()
                    .bg(rgb(if snapshot.selected == Some(id) {
                        0x365849
                    } else {
                        0x29424b
                    }))
                    .rounded_md()
                    .child(
                        div()
                            .id(ElementId::NamedInteger("tab".into(), id.value()))
                            .px_3()
                            .py_2()
                            .cursor_pointer()
                            .child(label)
                            .on_click(move |_, _, _| {
                                drop(select.tab(id).select());
                            })
                            .inspect_with(|| format!("rake.tab.select.{id}")),
                    )
                    .child(
                        div()
                            .id(ElementId::NamedInteger("close".into(), id.value()))
                            .px_2()
                            .py_2()
                            .cursor_pointer()
                            .child("×")
                            .on_click(move |_, _, _| {
                                drop(close.tab(id).close());
                            })
                            .inspect_with(|| format!("rake.tab.close.{id}")),
                    )
                    .inspect_with(|| format!("rake.tab.{id}"))
            }))
            .inspect("rake.tab-strip")
    }
}

/// Displays only the selected current view. Hidden tabs and history retain their entities.
#[derive(IntoElement)]
pub struct RouteOutlet<R: Route> {
    router: Router<R>,
    window: WindowId,
}

impl<R: Route> RouteOutlet<R> {
    /// Creates a selected-entry outlet. Its owning shell observes router changes.
    pub fn new(router: Router<R>) -> Self {
        let window = router.navigator().window_id();

        Self { router, window }
    }
}

impl<R: Route> RouteOutlet<R> {
    /// Presents this host's independently selected retained view.
    pub fn in_window(mut self, window: WindowId) -> Self {
        self.window = window;

        self
    }
}

impl<R: Route> RenderOnce for RouteOutlet<R> {
    fn render(self, _window: &mut Window, _cx: &mut gpui::App) -> impl IntoElement {
        let view = self
            .router
            .resource(self.window)
            .and_then(|resource| resource.downcast_ref::<AnyView>().cloned());

        #[cfg(feature = "devtools")]
        let snapshot = self.router.presented_snapshot(self.window);

        let outlet = div()
            .size_full()
            .children(view)
            .inspect("rake.route-outlet");

        #[cfg(feature = "devtools")]
        {
            if let Some(tab) = snapshot
                .tabs
                .iter()
                .find(|tab| Some(tab.id) == snapshot.selected)
            {
                outlet.owner(tab.id, tab.entries[tab.cursor].id)
            } else {
                outlet
            }
        }
        #[cfg(not(feature = "devtools"))]
        {
            outlet
        }
    }
}

#[cfg(not(target_family = "wasm"))]
type DriverTask = gpui::Task<()>;

#[cfg(target_family = "wasm")]
struct DriverTask(futures::future::AbortHandle);

#[cfg(target_family = "wasm")]
impl Drop for DriverTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(crate) struct Workspace<R: Route> {
    chrome: bool,
    router: Router<R>,
    restart: Option<R>,
    _driver: Option<DriverTask>,
    window: WindowId,
    #[cfg(target_family = "wasm")]
    _browser: super::web::browser::BrowserHistory,
}

#[cfg(feature = "devtools")]
impl<R: Route> Drop for Workspace<R> {
    fn drop(&mut self) {
        self.router.client().inspection().detach_window(self.window);
    }
}

pub(crate) struct RoutingRoot<R: Route> {
    pub(crate) chrome: bool,
    pub(crate) quit_guards: Vec<super::runtime::QuitGuard>,
    pub(crate) views: Rc<dyn RouteViews<R>>,
    pub(crate) initial: Option<R>,
    pub(crate) checkpoint: Option<Result<crate::reload::WorkspaceCheckpoint, String>>,
    pub(crate) config: RouterConfig,
    pub(crate) web: WebConfig,
    pub(crate) guards: Vec<Guard<R>>,
    pub(crate) control: crate::control::ControlConfig,
    pub(crate) commands: crate::control::Commands,
    pub(crate) application_id: String,
}

impl<R: Route> Workspace<R> {
    pub(crate) fn mount(
        services: Services,
        root: RoutingRoot<R>,
        window: &mut Window,
        cx: &mut gpui::App,
    ) -> Result<AnyView, MountError> {
        #[cfg(not(target_family = "wasm"))]
        cx.set_quit_mode(gpui::QuitMode::Explicit);

        let RoutingRoot {
            views,
            initial,
            checkpoint,
            config,
            web,
            guards,
            control,
            commands,
            application_id,
            chrome,
            quit_guards,
        } = root;

        let initial_window = checkpoint
            .as_ref()
            .and_then(|saved| saved.as_ref().ok())
            .and_then(|saved| saved.windows.first())
            .map_or(WindowId(1), |window| window.id);

        let contexts = Rc::new(RefCell::new(BTreeMap::new()));

        let router = Router::with_views(
            config,
            services,
            views,
            initial_window,
            contexts.clone(),
            window,
            cx,
        );

        #[cfg(not(target_family = "wasm"))]
        let native = Rc::new(NativeWindows {
            app: cx.to_async(),
            navigator: router.navigator(),
            contexts,
            views: Rc::new(RefCell::new(BTreeMap::new())),
            chrome,
        });

        #[cfg(not(target_family = "wasm"))]
        router
            .window_host(native.clone())
            .map_err(|error| MountError(error.to_string()))?;

        for guard in quit_guards {
            router
                .quit_guard(move |context| guard(context))
                .map_err(|error| MountError(error.to_string()))?;
        }
        router.install_quit_action(cx);
        #[cfg(all(feature = "updates-sparkle", target_os = "macos"))]
        if let Err(error) = router.install_sparkle() {
            tracing::error!(%error,"native updater could not start");
        }
        router.client().set_commands(commands);

        #[cfg(feature = "devtools")]
        if control.inspection_enabled() {
            router.client().inspection().attach();
        }

        for guard in guards {
            router
                .entry_guard(move |context| guard(context))
                .map_err(|error| MountError(error.to_string()))?;
        }

        #[cfg(target_family = "wasm")]
        let browser = super::web::browser::BrowserHistory::attach(&router, web)
            .map_err(|error| MountError(error.to_string()))?;

        #[cfg(not(target_family = "wasm"))]
        let _ = web;

        let restart = initial.clone().or_else(|| R::from_uri("/").ok());

        if checkpoint.is_none()
            && let Some(initial) = initial
        {
            drop(router.navigator().restore_destination(initial));
        }

        Ok(cx
            .new(|cx: &mut Context<Self>| {
                let driven = router.clone();
                let observed = router.clone();
                let client = router.client();
                let restore_id = application_id.clone();

                #[cfg(not(target_family = "wasm"))]
                let closing = client.clone();

                #[cfg(not(target_family = "wasm"))]
                let observed_views = native.views.clone();

                let drive = async move |_view: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                    let execute = async {
                        if let Some(checkpoint) = checkpoint {
                            match checkpoint {
                                Ok(checkpoint) => {
                                    match driven.restore_workspace(&checkpoint, &restore_id).await {
                                        Ok(()) => crate::reload::host::clear(),
                                        Err(error) => driven.reload_failed(error.to_string()),
                                    }
                                }
                                Err(error) => driven.reload_failed(error),
                            }
                        }

                        if let Err(error) = driven.run().await {
                            crate::error!(%error, "Router driver stopped");
                        }
                    };

                    let observe = async {
                        let mut revision = u64::MAX;

                        loop {
                            #[cfg(all(feature="updates",not(target_family="wasm")))]
                            let changed=Box::pin(observed.changed_with_updates(revision));
                            #[cfg(all(
                                not(feature="updates"),
                                feature="native-lifecycle",
                                target_os="macos"
                            ))]
                            let changed=Box::pin(observed.changed_with_native_quit(revision));
                            #[cfg(not(any(
                                all(feature="updates",not(target_family="wasm")),
                                all(
                                    not(feature="updates"),
                                    feature="native-lifecycle",
                                    target_os="macos"
                                )
                            )))]
                            let changed=Box::pin(observed.changed(revision));
                            #[cfg(feature="devtools")]
                            {
                                let inspection=observed.client().inspection().listen();
                                if let futures::future::Either::Left((next,_))=futures::future::select(changed,Box::pin(inspection)).await {revision=next;}
                            }
                            #[cfg(not(feature="devtools"))]
                            {revision=changed.await;}

                            #[cfg(not(target_family = "wasm"))]
                            if observed.quit_status().phase == crate::quit::QuitPhase::Ready {
                                closing.responses_idle().await;
                                #[cfg(all(feature="native-lifecycle",target_os="macos"))]
                                observed.prepare_native_exit(true);
                                #[cfg(feature="updates")]
                                match observed.complete_update_restart() {
                                    Ok(rake_updates::RestartDisposition::ProviderHandlesExit)=>continue,
                                    Ok(rake_updates::RestartDisposition::ExitHost)=>{},
                                    Err(error)=>{
                                        #[cfg(all(feature="native-lifecycle",target_os="macos"))]
                                        {observed.reply_native_quit(false);observed.prepare_native_exit(true);}
                                        tracing::error!(%error,"update handoff failed; manual restart required");
                                        let _=_view.update(cx,|_,cx|cx.notify());
                                        break;
                                    }
                                }
                                #[cfg(all(feature="native-lifecycle",target_os="macos"))]
                                observed.reply_native_quit(true);
                                cx.update(|cx| cx.quit());

                                break;
                            }

                            #[cfg(target_family = "wasm")]
                            if _view.update(cx, |_, cx| cx.notify()).is_err() {
                                break;
                            }

                            #[cfg(not(target_family = "wasm"))]
                            for view in observed_views.borrow().values() {
                                let _ = view.update(cx, |_, cx| cx.notify());
                            }
                        }
                    };

                    let control = crate::control::run_host(control, client, application_id);
                    let host = futures::future::select(Box::pin(execute), Box::pin(observe));

                    futures::future::select(Box::pin(host), Box::pin(control)).await;
                };

                #[cfg(not(target_family = "wasm"))]
                let driver = {
                    native
                        .views
                        .borrow_mut()
                        .insert(initial_window, cx.weak_entity());
                    let task = cx.spawn(drive);

                    cx.set_global(NativeDriver::<R> {
                        _task: task,
                        _windows: native,
                    });

                    None
                };

                #[cfg(target_family = "wasm")]
                let driver = {
                    let view = cx.weak_entity();
                    let mut context = cx.to_async();
                    let (abort, registration) = futures::future::AbortHandle::new_pair();

                    // GPUI's Web dispatcher uses chained timers, which Chrome can
                    // throttle in hidden tabs. Runtime I/O must wake via microtasks.
                    wasm_bindgen_futures::spawn_local(async move {
                        let _ = futures::future::Abortable::new(
                            drive(view, &mut context),
                            registration,
                        )
                        .await;
                    });

                    Some(DriverTask(abort))
                };

                Self {
                    chrome,
                    router,
                    restart,
                    _driver: driver,
                    window: initial_window,
                    #[cfg(target_family = "wasm")]
                    _browser: browser,
                }
            })
            .into())
    }
}

impl<R: Route> Render for Workspace<R> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = self.router.presented_snapshot(self.window);
        let navigator = self.router.navigator().for_window(self.window);

        if matches!(
            snapshot.quit.phase,
            crate::quit::QuitPhase::Stopping | crate::quit::QuitPhase::Ready
        ) {
            #[cfg(feature = "updates")]
            if let Some(error) = self.router.update_restart_error() {
                return div().size_full().bg(rgb(0x18232b)).text_color(rgb(0xf0f4e8)).p_4()
                    .child(format!("The update could not restart the app: {error}. The app finished shutting down. Restart manually to continue."))
                    .child(div().id("update-quit-manually").px_3().py_2().cursor_pointer().child("Quit").on_click(|_,_,cx|cx.quit()))
                    .inspect("rake.update.restart_error")
                    .root(crate::__rake_inspection!(self.router.client().inspection(),crate::inspection::Store::default()),snapshot.revision,self.window);
            }
            return div()
                .size_full()
                .bg(rgb(0x18232b))
                .text_color(rgb(0xf0f4e8))
                .p_4()
                .child(if snapshot.quit.phase == crate::quit::QuitPhase::Ready {
                    "Application closed. You can close this page."
                } else {
                    "Stopping managed work…"
                })
                .inspect("rake.quit")
                .root(
                    crate::__rake_inspection!(
                        self.router.client().inspection(),
                        crate::inspection::Store::default()
                    ),
                    snapshot.revision,
                    self.window,
                );
        }

        if snapshot.reload.request.is_some() {
            let router = self.router.clone();

            if snapshot.reload.phase == crate::reload::ReloadPhase::Quiescing {
                let host = self.window;

                window.on_next_frame(move |_, _| router.acknowledge_reload_ui(host));
                cx.notify();
            }

            return div()
                .size_full()
                .bg(rgb(0x18232b))
                .text_color(rgb(0xf0f4e8))
                .p_4()
                .child("Preparing development reload…")
                .inspect("rake.reload")
                .root(
                    crate::__rake_inspection!(
                        self.router.client().inspection(),
                        crate::inspection::Store::default()
                    ),
                    snapshot.revision,
                    self.window,
                );
        }

        if snapshot.reload.phase == crate::reload::ReloadPhase::RestoreFailed {
            let error = snapshot.reload.error.unwrap_or_default();
            let restart = self.restart.clone();
            let router = self.router.clone();

            return div()
                .size_full()
                .bg(rgb(0x18232b))
                .text_color(rgb(0xf0f4e8))
                .p_4()
                .child(format!(
                    "Workspace restoration failed: {error}. State is retained for the next build."
                ))
                .child(
                    div()
                        .id("restart-workspace")
                        .p_3()
                        .cursor_pointer()
                        .child("Restart with an empty workspace")
                        .on_click(move |_, _, _| {
                            crate::reload::host::clear();

                            router.clear_reload_error();

                            if let Some(initial) = restart.clone() {
                                drop(router.navigator().restore_destination(initial));
                            }
                        })
                        .inspect("rake.restart-workspace"),
                )
                .inspect("rake.restore-failed")
                .root(
                    crate::__rake_inspection!(
                        self.router.client().inspection(),
                        crate::inspection::Store::default()
                    ),
                    snapshot.revision,
                    self.window,
                );
        }

        let mut decision_area = div().flex().flex_col().gap_2();

        for decision in snapshot.decisions {
            let choices = decision.choices.into_iter().map(|choice| {
                let navigator = navigator.clone();
                let id = decision.id;
                #[cfg(feature = "devtools")]
                let semantic_id = format!("rake.decision.{id}.{}", choice.id);

                div()
                    .id(format!("{id}-{}", choice.id))
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(rgb(0x365849))
                    .cursor_pointer()
                    .child(choice.label)
                    .on_click(move |_, _, _| {
                        let _ = navigator.respond(id, &choice.id);
                    })
                    .inspect(crate::__rake_inspection!(semantic_id, ""))
            });

            decision_area = decision_area.child(
                div()
                    .id(format!("rake.decision.{}", decision.id))
                    .flex()
                    .gap_3()
                    .p_3()
                    .bg(rgb(0x59462c))
                    .child(decision.message)
                    .children(choices)
                    .inspect_with(|| format!("rake.decision.{}", decision.id)),
            );
        }

        let failure = snapshot
            .operations
            .last()
            .and_then(|(_, state)| match state {
                OperationState::Failed(error) => Some(error.to_string()),
                OperationState::Blocked(reason) => Some(format!("Operation blocked: {reason:?}")),
                _ => None,
            });

        let quitting = snapshot.quit.phase.is_pending();
        let quit_router = self.router.clone();
        let cancel_router = self.router.clone();
        let cancel_jobs_router = self.router.clone();
        let quit_id = snapshot.quit.operation_id;

        let quit_controls = div()
            .flex()
            .gap_3()
            .p_2()
            .child(
                div()
                    .id("quit-application")
                    .px_3()
                    .py_2()
                    .cursor_pointer()
                    .child(if quitting {
                        "Cancel remaining jobs"
                    } else {
                        "Quit"
                    })
                    .on_click(move |_, _, _| {
                        if quitting {
                            // The first quit policy is stable; cancel individual jobs explicitly.
                            let status = quit_router.quit_status();

                            for task in status.pending_tasks {
                                let _ = quit_router.cancel_task(task);
                            }

                            for operation in status.pending_operations {
                                let _ = quit_router.cancel(operation);
                            }
                        } else {
                            drop(quit_router.quit(crate::quit::QuitOptions::default()));
                        }
                    })
                    .inspect("rake.quit.request"),
            )
            .when(!quitting, |row| {
                row.child(
                    div()
                        .id("quit-cancel-jobs")
                        .px_3()
                        .py_2()
                        .cursor_pointer()
                        .child("Quit and cancel jobs")
                        .on_click(move |_, _, _| {
                            drop(cancel_jobs_router.quit(crate::quit::QuitOptions {
                                jobs: crate::quit::QuitJobs::Cancel,
                            }));
                        })
                        .inspect("rake.quit.cancel-jobs"),
                )
            })
            .when(quitting, |row| {
                row.child(format!(
                    "Quitting: {:?} · {} jobs · {} operations",
                    snapshot.quit.phase,
                    snapshot.quit.pending_tasks.len(),
                    snapshot.quit.pending_operations.len()
                ))
                .child(
                    div()
                        .id("cancel-quit")
                        .px_3()
                        .py_2()
                        .cursor_pointer()
                        .child("Cancel quit")
                        .on_click(move |_, _, _| {
                            if let Some(id) = quit_id {
                                let _ = cancel_router.cancel(id);
                            }
                        })
                        .inspect("rake.quit.cancel"),
                )
            });

        #[cfg(all(feature = "updates", not(target_family = "wasm")))]
        let quit_controls = if let Some(updates) = self.router.updates() {
            use rake_updates::UpdateState;
            let state = updates.state();
            let label = match &state {
                UpdateState::Available(info) => Some(format!("Update {} available…", info.version)),
                UpdateState::Downloading { progress, .. } if progress.total_bytes > 0 => {
                    Some(format!(
                        "Downloading {}% — Cancel",
                        progress
                            .received_bytes
                            .saturating_mul(100)
                            .checked_div(progress.total_bytes)
                            .unwrap_or(0)
                            .min(100)
                    ))
                }
                UpdateState::Downloading { .. } => Some("Downloading… — Cancel".into()),
                UpdateState::Checking => Some("Checking… — Cancel".into()),
                UpdateState::Idle | UpdateState::UpToDate => Some("Check for updates…".into()),
                UpdateState::Extracting { .. } => Some("Preparing update…".into()),
                UpdateState::ReadyToRestart(_) => Some("Restart to update".into()),
                UpdateState::Cancelling => Some("Cancelling update…".into()),
                UpdateState::AwaitingApproval(_) => Some("Waiting to restart…".into()),
                UpdateState::Restarting(_) => Some("Restarting for update…".into()),
                UpdateState::Failed { .. } => Some("Update failed — Retry".into()),
                UpdateState::ManagedExternally(_) => Some("Updates managed by your system".into()),
                _ => None,
            };
            let actionable = matches!(
                state,
                UpdateState::Idle
                    | UpdateState::UpToDate
                    | UpdateState::Available(_)
                    | UpdateState::ReadyToRestart(_)
                    | UpdateState::Checking
                    | UpdateState::Downloading { .. }
                    | UpdateState::Failed { .. }
            );
            let router = self.router.clone();
            if let Some(label) = label {
                quit_controls.child(div().id("update-status").px_3().py_2().child(label).when(
                    actionable,
                    |control| {
                        control.cursor_pointer().on_click(move |_, _, _| {
                            let result = match updates.state() {
                                UpdateState::Available(_) => router.download_update(),
                                UpdateState::ReadyToRestart(_) => router
                                    .restart_for_update(crate::quit::QuitOptions::default())
                                    .map(|_| ()),
                                UpdateState::Checking | UpdateState::Downloading { .. } => {
                                    updates.cancel();
                                    Ok(())
                                }
                                UpdateState::Idle
                                | UpdateState::UpToDate
                                | UpdateState::Failed { .. } => router.check_for_updates(),
                                _ => Ok(()),
                            };
                            if let Err(error) = result {
                                tracing::error!(%error, "update action failed");
                            }
                        })
                    },
                ))
            } else {
                quit_controls
            }
        } else {
            quit_controls
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x18232b))
            .text_color(rgb(0xf0f4e8))
            .when(self.chrome, |root| {
                root.child(
                    crate::inspection::inspect(
                        TabBar::new(self.router.clone()).in_window(self.window),
                    )
                    .semantic_id("rake.tabs"),
                )
                .child(quit_controls)
            })
            .child(decision_area.inspect("rake.decisions"))
            .children(failure)
            .children(snapshot.history_error)
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .child(RouteOutlet::new(self.router.clone()).in_window(self.window))
                    .inspect("rake.content"),
            )
            .inspect("rake.workspace")
            .root(
                crate::__rake_inspection!(
                    self.router.client().inspection(),
                    crate::inspection::Store::default()
                ),
                snapshot.revision,
                self.window,
            )
    }
}

#[cfg(not(target_family = "wasm"))]
struct NativeDriver<R: Route> {
    _task: gpui::Task<()>,
    _windows: Rc<NativeWindows<R>>,
}

#[cfg(not(target_family = "wasm"))]
impl<R: Route> gpui::Global for NativeDriver<R> {}

#[cfg(not(target_family = "wasm"))]
struct NativeWindows<R: Route> {
    app: gpui::AsyncApp,
    navigator: Navigator<R>,
    contexts: Rc<RefCell<BTreeMap<WindowId, gpui::AsyncWindowContext>>>,
    views: Rc<RefCell<BTreeMap<WindowId, gpui::WeakEntity<Workspace<R>>>>>,
    chrome: bool,
}

#[cfg(not(target_family = "wasm"))]
impl<R: Route> WindowHost for Rc<NativeWindows<R>> {
    fn open(
        &self,
        id: WindowId,
        activation: Activation,
    ) -> futures::future::LocalBoxFuture<'static, Result<(), MountError>> {
        let host = self.clone();

        async move {
            let router = host
                .navigator
                .router()
                .ok_or_else(|| MountError("Runtime closed.".into()))?;

            let app = host.app.clone();

            app.update(|cx| {
                cx.open_window(
                    gpui::WindowOptions {
                        focus: activation == Activation::Activate,
                        ..gpui::WindowOptions::default()
                    },
                    move |window, cx| {
                        router
                            .attach_window(id, window, cx)
                            .expect("reserved native window");
                        let view = cx.new(|_| Workspace {
                            chrome: host.chrome,
                            restart: R::from_uri("/").ok(),
                            router,
                            _driver: None,
                            window: id,
                        });

                        host.views.borrow_mut().insert(id, view.downgrade());

                        view
                    },
                )
                .map(|_| ())
                .map_err(|error| MountError(error.to_string()))
            })
        }
        .boxed_local()
    }

    fn close(&self, id: WindowId) {
        let context = self.contexts.borrow_mut().remove(&id);

        self.views.borrow_mut().remove(&id);

        if let Some(mut context) = context {
            let _ = context.update(|window, _| window.remove_window());
        }
    }

    fn activate(&self, id: WindowId) {
        let context = self.contexts.borrow().get(&id).cloned();

        if let Some(mut context) = context {
            let _ = context.update(|window, _| window.activate_window());
        }
    }
}
