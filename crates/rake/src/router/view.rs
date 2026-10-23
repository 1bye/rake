//! GPUI mounting and presentation; retained entries own independent view entities.

use crate::inspection::InspectElement as _;

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{AnyView, Context, ElementId, IntoElement, Render, RenderOnce, Task, Window, div, rgb};

use crate::Services;
use crate::tasks::{ActivityRun, AppHandle, TaskError, TaskHandle};

use super::runtime::{Guard, Mount, Resource};

use super::{
    EntryHandle, MountError, Navigator, OperationState, Route, Router, RouterConfig, TabHandle,
    TabId, WebConfig,
};

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
        Self::with_views(config, services, Rc::new(views), window, cx)
    }

    fn with_views(
        config: RouterConfig,
        services: Services,
        views: Rc<dyn RouteViews<R>>,
        window: &Window,
        cx: &gpui::App,
    ) -> Self {
        let context = window.to_async(cx);
        let collector = services.diagnostics();

        let mount: Mount<R> = Rc::new(move |destination, entry, navigator| {
            let mut context = context.clone();
            let views = views.clone();
            let tab_id = entry.tab_id();
            let services = services.clone();

            Box::pin(async move {
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

        router.observe_host(window, cx);

        router
    }
}

/// A default application tab strip driven by the authoritative shared router.
#[derive(IntoElement)]
pub struct TabBar<R: Route> {
    router: Router<R>,
}

impl<R: Route> TabBar<R> {
    /// Creates a strip. Its owning view redraws when `Router::changed` resolves.
    pub fn new(router: Router<R>) -> Self {
        Self { router }
    }
}

impl<R: Route> RenderOnce for TabBar<R> {
    fn render(self, _window: &mut Window, _cx: &mut gpui::App) -> impl IntoElement {
        let snapshot = self.router.snapshot();
        let navigator = self.router.navigator();

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
                            .inspect(format!("rake.tab.select.{id}")),
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
                            .inspect(format!("rake.tab.close.{id}")),
                    )
                    .inspect(format!("rake.tab.{id}"))
            }))
            .inspect("rake.tab-strip")
    }
}

/// Displays only the selected current view. Hidden tabs and history retain their entities.
#[derive(IntoElement)]
pub struct RouteOutlet<R: Route> {
    router: Router<R>,
}

impl<R: Route> RouteOutlet<R> {
    /// Creates a selected-entry outlet. Its owning shell observes router changes.
    pub fn new(router: Router<R>) -> Self {
        Self { router }
    }
}

impl<R: Route> RenderOnce for RouteOutlet<R> {
    fn render(self, _window: &mut Window, _cx: &mut gpui::App) -> impl IntoElement {
        let view = self
            .router
            .resource()
            .and_then(|resource| resource.downcast_ref::<AnyView>().cloned());

        let snapshot = self.router.snapshot();

        let outlet = div()
            .size_full()
            .children(view)
            .inspect("rake.route-outlet");

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
}

pub(crate) struct Workspace<R: Route> {
    router: Router<R>,
    restart: Option<R>,
    _driver: Task<()>,
    #[cfg(target_family = "wasm")]
    _browser: super::web::browser::BrowserHistory,
}

pub(crate) struct RoutingRoot<R: Route> {
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
        } = root;

        let router = Router::with_views(config, services, views, window, cx);

        router.client().set_commands(commands);

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

                let driver = cx.spawn(async move |view, cx| {
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
                            revision = observed.changed(revision).await;

                            if view.update(cx, |_, cx| cx.notify()).is_err() {
                                break;
                            }
                        }
                    };

                    let control = crate::control::run_host(control, client, application_id);
                    let host = futures::future::select(Box::pin(execute), Box::pin(observe));

                    futures::future::select(Box::pin(host), Box::pin(control)).await;
                });

                Self {
                    router,
                    restart,
                    _driver: driver,
                    #[cfg(target_family = "wasm")]
                    _browser: browser,
                }
            })
            .into())
    }
}

impl<R: Route> Render for Workspace<R> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = self.router.snapshot();
        let navigator = self.router.navigator();

        if snapshot.reload.request.is_some() {
            let router = self.router.clone();

            if snapshot.reload.phase == crate::reload::ReloadPhase::Quiescing {
                window.on_next_frame(move |_, _| router.acknowledge_reload_ui());
                cx.notify();
            }

            return div()
                .size_full()
                .bg(rgb(0x18232b))
                .text_color(rgb(0xf0f4e8))
                .p_4()
                .child("Preparing development reload…")
                .inspect("rake.reload")
                .root(self.router.client().inspection(), snapshot.revision);
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
                .root(self.router.client().inspection(), snapshot.revision);
        }

        let mut decision_area = div().flex().flex_col().gap_2();

        for decision in snapshot.decisions {
            let choices = decision.choices.into_iter().map(|choice| {
                let navigator = navigator.clone();
                let id = decision.id;
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
                    .inspect(semantic_id)
            });

            decision_area = decision_area.child(
                div()
                    .flex()
                    .gap_3()
                    .p_3()
                    .bg(rgb(0x59462c))
                    .child(decision.message)
                    .children(choices)
                    .inspect(format!("rake.decision.{}", decision.id)),
            );
        }

        let failure = snapshot
            .operations
            .last()
            .and_then(|(_, state)| match state {
                OperationState::Failed(error) => Some(error.to_string()),
                OperationState::Blocked(reason) => Some(format!("Navigation blocked: {reason:?}")),
                _ => None,
            });

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x18232b))
            .text_color(rgb(0xf0f4e8))
            .child(
                crate::inspection::inspect(TabBar::new(self.router.clone()))
                    .semantic_id("rake.tabs"),
            )
            .child(decision_area.inspect("rake.decisions"))
            .children(failure)
            .children(snapshot.history_error)
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .child(RouteOutlet::new(self.router.clone()))
                    .inspect("rake.content"),
            )
            .inspect("rake.workspace")
            .root(self.router.client().inspection(), snapshot.revision)
    }
}
