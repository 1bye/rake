//! Covers: rendered navigation progress, cancellation, retry admission and
//! retained failures. Native host observation is intentionally omitted because
//! GPUI's test window has no platform handle.
//! Requires: view-test-support; no display or native window.
#![cfg(all(feature = "view-test-support", not(target_family = "wasm")))]

use crate::diagnostics::Collector;
use crate::gpui::{self, AnyView, Context, InputEvent as _, Render, Window, prelude::*};
use crate::router::runtime::{Mount, Resource};
use crate::router::{
    CancelReason, MountError, NavigationState, NavigationStatus, OpenTarget, OperationError,
    Outcome, Router, RouterConfig, WindowId,
};
use futures::FutureExt as _;
use futures::channel::oneshot;
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
}

impl Render for Screen {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::div().child(self.label)
    }
}

struct BoundaryHost {
    router: Router<Destination>,
    _driver: gpui::Task<()>,
}

impl Render for BoundaryHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        crate::router::NavigationBoundary::new(self.router.clone())
    }
}

fn add_host(
    cx: &mut gpui::TestAppContext,
    mount: Mount<Destination>,
) -> gpui::WindowHandle<BoundaryHost> {
    cx.add_window(move |_, cx| {
        let router = Router::with_mount(RouterConfig::default(), Collector::default(), mount);
        let driven = router.clone();
        let driver = cx.spawn(async move |_, _| driven.run().await.unwrap());
        BoundaryHost {
            router,
            _driver: driver,
        }
    })
}

fn router_for(
    cx: &gpui::TestAppContext,
    window: gpui::WindowHandle<BoundaryHost>,
) -> Router<Destination> {
    window.read_with(cx, |host, _| host.router.clone()).unwrap()
}

fn draw(cx: &mut gpui::TestAppContext, window: gpui::WindowHandle<BoundaryHost>) {
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.draw(cx).clear(cx);
    })
    .unwrap();
}

fn activate_first_control(cx: &mut gpui::TestAppContext, window: gpui::WindowHandle<BoundaryHost>) {
    cx.update_window(window.into(), |_, window, cx| window.focus_next(cx))
        .unwrap();
    cx.run_until_parked();
    draw(cx, window);
    key_press(cx, window, "enter");
}

fn focus_first_control(cx: &mut gpui::TestAppContext, window: gpui::WindowHandle<BoundaryHost>) {
    cx.update_window(window.into(), |_, window, cx| window.focus_next(cx))
        .unwrap();
    cx.run_until_parked();
    draw(cx, window);
}

fn key_press(cx: &mut gpui::TestAppContext, window: gpui::WindowHandle<BoundaryHost>, key: &str) {
    let keystroke = gpui::Keystroke::parse(key).unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        window.dispatch_event(
            gpui::KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(gpui::KeyUpEvent { keystroke }.to_platform_input(), cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn stop(
    cx: &mut gpui::TestAppContext,
    window: gpui::WindowHandle<BoundaryHost>,
    router: Router<Destination>,
) {
    router.shutdown();
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    drop(router);
}

fn failed_status(router: &Router<Destination>) -> NavigationStatus {
    router
        .navigation(WindowId(1))
        .into_iter()
        .find(|status| matches!(status.state, NavigationState::Failed(_)))
        .expect("retained failed navigation")
}

#[test]
fn pending_first_navigation_has_keyboard_cancel_and_cancels_operation() {
    let mut cx = gpui::TestAppContext::single();
    let mount: Mount<Destination> = Rc::new(|_, _, _| {
        async {
            futures::future::pending::<()>().await;
            Ok(Rc::new(()) as Resource)
        }
        .boxed_local()
    });
    let window = add_host(&mut cx, mount);
    let router = router_for(&cx, window);
    let operation = router
        .navigator()
        .open(Destination::Home, OpenTarget::NewTab);

    cx.run_until_parked();
    assert!(matches!(
        router.navigation(WindowId(1)).as_slice(),
        [NavigationStatus {
            state: NavigationState::Pending,
            ..
        }]
    ));
    draw(&mut cx, window);
    activate_first_control(&mut cx, window);

    assert!(matches!(
        operation.wait().now_or_never().unwrap(),
        Ok(Outcome::Cancelled(CancelReason::Requested))
    ));
    stop(&mut cx, window, router);
}

#[test]
fn failed_first_navigation_retries_single_flight_and_keeps_parent_hidden() {
    let mut cx = gpui::TestAppContext::single();
    let attempts = Rc::new(Cell::new(0));
    let retry_gate = Rc::new(RefCell::new(None::<oneshot::Receiver<()>>));
    let observed_attempts = attempts.clone();
    let observed_gate = retry_gate.clone();
    let mount: Mount<Destination> = Rc::new(move |_, _, _| {
        let attempt = observed_attempts.get() + 1;
        observed_attempts.set(attempt);
        let gate = observed_gate.borrow_mut().take();
        async move {
            if attempt == 1 {
                return Err(MountError("first load failed".into()));
            }
            if let Some(gate) = gate {
                let _ = gate.await;
            }
            Err(MountError("retry still failed".into()))
        }
        .boxed_local()
    });
    let window = add_host(&mut cx, mount);
    let router = router_for(&cx, window);
    let operation = router
        .navigator()
        .open(Destination::Home, OpenTarget::NewTab);
    cx.run_until_parked();
    assert!(matches!(
        operation.wait().now_or_never().unwrap(),
        Err(OperationError::Mount(_))
    ));
    let parent = failed_status(&router);
    draw(&mut cx, window);

    let (release, gate) = oneshot::channel();
    *retry_gate.borrow_mut() = Some(gate);
    activate_first_control(&mut cx, window);

    let statuses = router.navigation(WindowId(1));
    let retry = statuses
        .iter()
        .find(|status| status.retry_of.as_ref() == Some(&parent.key))
        .expect("retry status");
    assert!(matches!(retry.state, NavigationState::Pending));
    assert!(matches!(
        router.retry_navigation(&parent.key),
        Err(OperationError::InvalidOptions)
    ));
    assert_eq!(attempts.get(), 2);

    release.send(()).unwrap();
    cx.run_until_parked();
    let statuses = router.navigation(WindowId(1));
    assert!(statuses.iter().any(|status| {
        status.retry_of.as_ref() == Some(&parent.key)
            && matches!(
                status.state,
                NavigationState::Failed(OperationError::Mount(_))
            )
    }));
    // The parent remains retained for diagnostics, but the boundary suppresses
    // it whenever a retry child exists; a subsequent retry uses the child key.
    let child = statuses
        .iter()
        .find(|status| status.retry_of.as_ref() == Some(&parent.key))
        .unwrap();
    assert!(router.retry_navigation(&child.key).is_ok());
    cx.run_until_parked();
    draw(&mut cx, window);
    let before = attempts.get();
    // A failed first navigation has no page, so its retry chain keeps Retry
    // but never adds Dismiss. Tabbing from the sole control starts another try.
    focus_first_control(&mut cx, window);
    key_press(&mut cx, window, "tab");
    draw(&mut cx, window);
    key_press(&mut cx, window, "enter");
    assert!(
        attempts.get() > before,
        "initial failures expose Retry, not Dismiss"
    );
    stop(&mut cx, window, router);
}

#[test]
fn failed_replacement_keeps_existing_page_and_keyboard_dismisses_feedback() {
    let mut cx = gpui::TestAppContext::single();
    let rendered = Rc::new(RefCell::new(Vec::new()));
    let mounted = rendered.clone();
    let context_holder = Rc::new(RefCell::new(None::<gpui::AsyncWindowContext>));
    let mount_context = context_holder.clone();
    let mount: Mount<Destination> = Rc::new(move |destination, _, _| {
        if matches!(destination, Destination::Other) {
            return async { Err(MountError("replacement failed".into())) }.boxed_local();
        }
        let rendered = mounted.clone();
        let mut context = mount_context.borrow_mut().take();
        async move {
            let mut context = context
                .take()
                .ok_or_else(|| MountError("missing test window context".into()))?;
            let view = context
                .update(|_, cx| AnyView::from(cx.new(|_| Screen { label: "home" })))
                .map_err(|error| MountError(error.to_string()))?;
            rendered.borrow_mut().push("home");
            Ok(Rc::new(view) as Resource)
        }
        .boxed_local()
    });
    let context_setter = context_holder.clone();
    let window = cx.add_window(move |window, cx| {
        *context_setter.borrow_mut() = Some(window.to_async(cx));
        let router = Router::with_mount(RouterConfig::default(), Collector::default(), mount);
        let driven = router.clone();
        let driver = cx.spawn(async move |_, _| driven.run().await.unwrap());
        BoundaryHost {
            router,
            _driver: driver,
        }
    });
    let router = router_for(&cx, window);
    let home = router
        .navigator()
        .open(Destination::Home, OpenTarget::NewTab);
    cx.run_until_parked();
    assert!(matches!(
        home.wait().now_or_never().unwrap(),
        Ok(Outcome::Committed(_))
    ));

    let tab = router.navigator().tab(router.snapshot().selected.unwrap());
    let replacement = tab.navigate(Destination::Other);
    cx.run_until_parked();
    assert!(matches!(
        replacement.wait().now_or_never().unwrap(),
        Err(OperationError::Mount(_))
    ));
    draw(&mut cx, window);

    let snapshot = router.snapshot();
    assert_eq!(snapshot.tabs[0].entries.len(), 1);
    assert_eq!(snapshot.tabs[0].entries[0].destination.route, "home");
    let failed = failed_status(&router);
    assert!(
        failed.key.tab_id.is_some(),
        "replacement failure has an existing page"
    );

    // With an existing page, the rendered row has Retry followed by Dismiss.
    // Advancing once from Retry and activating the next focusable control must
    // dismiss only feedback; the retained Home entry remains selected.
    focus_first_control(&mut cx, window);
    key_press(&mut cx, window, "tab");
    draw(&mut cx, window);
    key_press(&mut cx, window, "enter");
    assert!(
        router
            .navigation(WindowId(1))
            .iter()
            .all(|status| status.key != failed.key)
    );
    assert_eq!(
        router.snapshot().tabs[0].entries[0].destination.route,
        "home"
    );
    assert!(rendered.borrow().contains(&"home"));
    stop(&mut cx, window, router);
}

#[test]
fn cancelled_retry_with_existing_page_remains_retryable() {
    let mut cx = gpui::TestAppContext::single();
    let rendered = Rc::new(RefCell::new(Vec::new()));
    let context_holder = Rc::new(RefCell::new(None::<gpui::AsyncWindowContext>));
    let mount_context = context_holder.clone();
    let other_attempts = Rc::new(Cell::new(0));
    let retry_gate = Rc::new(RefCell::new(None::<oneshot::Receiver<()>>));
    let observed_attempts = other_attempts.clone();
    let observed_gate = retry_gate.clone();
    let mounted = rendered.clone();
    let mount: Mount<Destination> = Rc::new(move |destination, _, _| {
        if matches!(destination, Destination::Other) {
            let attempt = observed_attempts.get() + 1;
            observed_attempts.set(attempt);
            let gate = if attempt == 2 {
                observed_gate.borrow_mut().take()
            } else {
                None
            };
            return async move {
                if let Some(gate) = gate {
                    let _ = gate.await;
                }
                Err(MountError("other unavailable".into()))
            }
            .boxed_local();
        }

        let mut context = mount_context.borrow_mut().take();
        let mounted = mounted.clone();
        async move {
            let mut context = context
                .take()
                .ok_or_else(|| MountError("missing test window context".into()))?;
            let view = context
                .update(|_, cx| AnyView::from(cx.new(|_| Screen { label: "home" })))
                .map_err(|error| MountError(error.to_string()))?;
            mounted.borrow_mut().push("home");
            Ok(Rc::new(view) as Resource)
        }
        .boxed_local()
    });
    let context_setter = context_holder.clone();
    let window = cx.add_window(move |window, cx| {
        *context_setter.borrow_mut() = Some(window.to_async(cx));
        let router = Router::with_mount(RouterConfig::default(), Collector::default(), mount);
        let driven = router.clone();
        let driver = cx.spawn(async move |_, _| driven.run().await.unwrap());
        BoundaryHost {
            router,
            _driver: driver,
        }
    });
    let router = router_for(&cx, window);
    let home = router
        .navigator()
        .open(Destination::Home, OpenTarget::NewTab);
    cx.run_until_parked();
    assert!(matches!(
        home.wait().now_or_never().unwrap(),
        Ok(Outcome::Committed(_))
    ));

    let tab = router.navigator().tab(router.snapshot().selected.unwrap());
    let replacement = tab.navigate(Destination::Other);
    cx.run_until_parked();
    assert!(matches!(
        replacement.wait().now_or_never().unwrap(),
        Err(OperationError::Mount(_))
    ));
    let parent = failed_status(&router);

    let (release, gate) = oneshot::channel();
    *retry_gate.borrow_mut() = Some(gate);
    draw(&mut cx, window);
    activate_first_control(&mut cx, window);
    let pending = router
        .navigation(WindowId(1))
        .into_iter()
        .find(|status| status.retry_of.as_ref() == Some(&parent.key))
        .expect("pending retry");
    assert!(matches!(pending.state, NavigationState::Pending));

    // A pending retry is represented by Cancel. Cancelling it must leave a
    // recoverable requested-cancelled child when the old Home page is visible.
    draw(&mut cx, window);
    activate_first_control(&mut cx, window);
    drop(release);
    let cancelled = router
        .navigation(WindowId(1))
        .into_iter()
        .find(|status| status.key == pending.key)
        .expect("cancelled retry");
    assert!(matches!(
        cancelled.state,
        NavigationState::Cancelled(CancelReason::Requested)
    ));
    assert_eq!(
        router.snapshot().tabs[0].entries[0].destination.route,
        "home"
    );

    // The cancelled child remains the visible recovery row. Retry through the
    // keyboard and require a fresh route-mount attempt rather than a stale no-op.
    draw(&mut cx, window);
    let before = other_attempts.get();
    activate_first_control(&mut cx, window);
    assert_eq!(other_attempts.get(), before + 1);
    assert!(router.navigation(WindowId(1)).iter().any(|status| {
        status.retry_of.as_ref() == Some(&cancelled.key)
            && matches!(
                status.state,
                NavigationState::Failed(OperationError::Mount(_))
            )
    }));
    assert!(rendered.borrow().contains(&"home"));
    stop(&mut cx, window, router);
}
