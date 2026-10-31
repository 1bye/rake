//! Covers: persistent layout identity, host-bound context, visible outlet updates,
//! and fallible layout construction without touching native window observation.
//! Real: GPUI entities, the router driver, retained route resources and layout
//! composition. Doubles: GPUI TestPlatform (it has no native window handle).
//! Requires: view-test-support; no display or native host.
#![cfg(all(feature = "view-test-support", not(target_family = "wasm")))]

use super::{LayoutContext, LayoutFactory, Workspace, mount_layout};
use crate::diagnostics::Collector;
use crate::gpui::{self, AnyView, Context, Entity, Render, Window, prelude::*};
use crate::http::{HttpClient, HttpConfig};
use crate::router::runtime::{Mount, Resource};
use crate::router::{MountError, RouteOutlet, Router, RouterConfig, WindowId};
use crate::services::{ServiceRegistry, Services};
use futures::FutureExt as _;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone, crate::Route)]
enum Destination {
    #[route(id = "home", path = "/")]
    Home,
    #[route(id = "other", path = "/other")]
    Other,
}

struct Screen {
    label: &'static str,
    rendered: Rc<RefCell<Vec<&'static str>>>,
}

impl Render for Screen {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.rendered.borrow_mut().push(self.label);
        gpui::div().child(self.label)
    }
}

struct LayoutProbe {
    context: LayoutContext<Destination>,
    outlet: RouteOutlet<Destination>,
    renders: Rc<RefCell<Vec<String>>>,
    local_state: usize,
}

impl Render for LayoutProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        if let Ok(router) = self.context.router() {
            let snapshot = router.presented_snapshot(self.context.window_id());
            if let Some(tab) = snapshot
                .tabs
                .iter()
                .find(|tab| Some(tab.id) == snapshot.selected)
            {
                self.renders
                    .borrow_mut()
                    .push(tab.entries[tab.cursor].destination.route.clone());
            }
        }

        gpui::div().size_full().child(self.outlet.clone())
    }
}

struct ErrorHost;

impl Render for ErrorHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::div()
    }
}

fn services() -> Services {
    Services::new(
        HttpClient::new(HttpConfig::new()).unwrap(),
        Collector::default(),
        ServiceRegistry::new(),
    )
}

fn draw(cx: &mut gpui::TestAppContext, window: gpui::WindowHandle<Workspace<Destination>>) {
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.draw(cx).clear(cx);
    })
    .unwrap();
}

#[test]
fn layout_is_created_once_and_outlet_updates_while_layout_state_survives() {
    let mut cx = gpui::TestAppContext::single();
    let route_renders = Rc::new(RefCell::new(Vec::new()));
    let layout_renders = Rc::new(RefCell::new(Vec::new()));
    let factory_calls = Rc::new(Cell::new(0));
    let layout_entity = Rc::new(RefCell::new(None::<Entity<LayoutProbe>>));
    let layout_context = Rc::new(RefCell::new(None::<LayoutContext<Destination>>));
    let app_services = services();

    let window = cx.add_window(|window, cx| {
        let context = window.to_async(cx);
        let route_renders = route_renders.clone();
        let mount: Mount<Destination> = Rc::new(move |destination, _, _| {
            let label = match destination {
                Destination::Home => "home",
                Destination::Other => "other",
            };
            let route_renders = route_renders.clone();
            let mut context = context.clone();

            async move {
                let view = context
                    .update(|_, cx| {
                        AnyView::from(cx.new(|_| Screen {
                            label,
                            rendered: route_renders,
                        }))
                    })
                    .map_err(|error| MountError(error.to_string()))?;
                Ok(Rc::new(view) as Resource)
            }
            .boxed_local()
        });
        let router = Router::with_mount(RouterConfig::default(), Collector::default(), mount);
        let factory_calls = factory_calls.clone();
        let layout_renders = layout_renders.clone();
        let layout_entity = layout_entity.clone();
        let layout_context = layout_context.clone();
        let factory: LayoutFactory<Destination> = Rc::new(move |context, _, cx| {
            factory_calls.set(factory_calls.get() + 1);
            *layout_context.borrow_mut() = Some(context.clone());
            let entity = cx.new(|_| LayoutProbe {
                context: context.clone(),
                outlet: context.outlet(),
                renders: layout_renders.clone(),
                local_state: 0,
            });
            *layout_entity.borrow_mut() = Some(entity.clone());
            Ok(entity.into())
        });
        let layout = mount_layout(
            Some(&factory),
            router.clone(),
            app_services.clone(),
            WindowId(1),
            window,
            cx,
        )
        .unwrap()
        .unwrap();
        let driven = router.clone();
        let driver = cx.spawn(async move |_, _| driven.run().await.unwrap());

        Workspace {
            chrome: false,
            router,
            restart: None,
            layout: Some(layout),
            _driver: Some(driver),
            window: WindowId(1),
        }
    });

    let router = window
        .read_with(&cx, |workspace, _| workspace.router.clone())
        .unwrap();
    let workspace = window.entity(&cx).unwrap().downgrade();
    let observed = router.clone();
    let observer = cx.spawn(async move |mut cx| {
        let mut revision = 0;
        loop {
            revision = observed.changed(revision).await;
            if !super::notify_workspace(&workspace, &mut cx) {
                break;
            }
        }
    });
    let open = router
        .navigator()
        .open(Destination::Home, crate::router::OpenTarget::NewTab);
    cx.run_until_parked();
    assert!(matches!(
        open.wait().now_or_never().unwrap(),
        Ok(crate::router::Outcome::Committed(_))
    ));
    draw(&mut cx, window);
    assert_eq!(factory_calls.get(), 1);
    assert!(layout_renders.borrow().iter().any(|route| route == "home"));
    assert!(route_renders.borrow().contains(&"home"));

    let layout = layout_entity.borrow().as_ref().unwrap().clone();
    layout.update(&mut cx, |layout, _| layout.local_state = 41);
    let tab = router.navigator().tab(router.snapshot().selected.unwrap());
    let visit = tab.navigate(Destination::Other);
    cx.run_until_parked();
    assert!(matches!(
        visit.wait().now_or_never().unwrap(),
        Ok(crate::router::Outcome::Committed(_))
    ));
    draw(&mut cx, window);

    assert_eq!(
        factory_calls.get(),
        1,
        "route changes must not recreate the layout"
    );
    assert_eq!(cx.read(|cx| layout.read(cx).local_state), 41);
    assert!(layout_renders.borrow().iter().any(|route| route == "other"));
    assert!(route_renders.borrow().contains(&"other"));
    router.shutdown();
    cx.run_until_parked();
    drop(observer);
    cx.update_window(window.into(), |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    drop(layout);
    layout_entity.borrow_mut().take();
    drop(router);
    assert!(layout_context.borrow().as_ref().unwrap().router().is_err());
}

#[test]
fn layout_context_is_window_bound_and_factory_errors_are_mount_errors() {
    let mut cx = gpui::TestAppContext::single();
    let seen_window = Rc::new(Cell::new(None));
    let app_services = services();
    let seen_window_for_factory = seen_window.clone();
    let factory: LayoutFactory<Destination> = Rc::new(move |context, _, _| {
        seen_window_for_factory.set(Some(context.window_id()));
        assert_eq!(context.outlet().window, context.window_id());
        Err(MountError("layout unavailable".into()))
    });
    let error = Rc::new(RefCell::new(None));
    let error_for_window = error.clone();
    let window = cx.add_window(|window, cx| {
        let router = Router::new(RouterConfig::default());
        let before = router.snapshot();
        *error_for_window.borrow_mut() = mount_layout(
            Some(&factory),
            router.clone(),
            app_services.clone(),
            WindowId(7),
            window,
            cx,
        )
        .unwrap_err()
        .into();
        let after = router.snapshot();
        assert_eq!(before.revision, after.revision);
        assert_eq!(before.windows.len(), after.windows.len());
        ErrorHost
    });

    assert_eq!(seen_window.get(), Some(WindowId(7)));
    assert_eq!(
        error.borrow().as_ref().map(|error| error.0.as_str()),
        Some("layout unavailable")
    );
    cx.update_window(window.into(), |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
}
