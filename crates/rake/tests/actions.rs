//! Covers: public initiated actions, finite-work shutdown, typed errors and owner cancellation.
//! Real: Rake scheduler and GPUI entity ownership. Doubles: GPUI TestPlatform.
//! Requires: view-test-support; no native display/GPU or network.

#![cfg(all(feature = "view-test-support", not(target_family = "wasm")))]

use rake::gpui::{self, prelude::*};
use rake::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

#[derive(Debug, thiserror::Error)]
#[error("source failed")]
struct Failure;

struct Host {
    action: Option<AsyncAction<String, Failure>>,
    runtime: RuntimeContext,
    _driver: gpui::Task<()>,
}

impl Render for Host {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::div()
    }
}

fn mount<F: std::future::Future<Output = Result<String, Failure>> + 'static>(
    app: &mut gpui::TestAppContext,
    future: F,
) -> gpui::WindowHandle<Host> {
    app.add_window(|_, cx| {
        let services = Services::from_config(
            HttpConfig::new(),
            Collector::default(),
            ServiceRegistry::new(),
        )
        .unwrap();

        let runtime = Runtime::new(RouterConfig::default(), services);
        let context = runtime.context();
        let action = context.action(cx, future);
        let driver = cx.spawn(async move |_, _| runtime.run().await.unwrap());

        Host {
            action: Some(action),
            runtime: context,
            _driver: driver,
        }
    })
}

#[test]
fn initiated_action_settles_before_normal_quit_commits() {
    let mut app = gpui::TestAppContext::single();
    let (send, receive) = futures::channel::oneshot::channel();
    let host = mount(&mut app, async move { receive.await.map_err(|_| Failure) });

    app.run_until_parked();
    let quit = host
        .read_with(&app, |host, _| host.runtime.quit(QuitOptions::default()))
        .unwrap();

    app.run_until_parked();
    host.read_with(&app, |host, cx| {
        assert!(host.action.as_ref().unwrap().is_running(cx));
        assert_eq!(
            host.runtime.quit_status().unwrap().phase,
            QuitPhase::WaitingForJobs
        );
    })
    .unwrap();
    send.send("saved".into()).unwrap();
    app.run_until_parked();
    assert!(matches!(
        futures::executor::block_on(quit.wait()).unwrap(),
        Outcome::Committed(_)
    ));
    host.read_with(&app, |host, cx| assert!(matches!(host.action.as_ref().unwrap().state(cx), LoadStatus::Ready(value) if value == "saved"))).unwrap();
}

#[test]
fn initiated_action_keeps_source_errors_typed() {
    let mut app = gpui::TestAppContext::single();
    let host = mount(&mut app, async { Err(Failure) });

    app.run_until_parked();
    host.read_with(&app, |host, cx| {
        assert!(matches!(
            host.action.as_ref().unwrap().state(cx),
            LoadStatus::Failed(rake::loading::LoadError::Source(Failure))
        ))
    })
    .unwrap();
}

#[test]
fn dropping_the_action_drops_its_pending_future() {
    struct Dropped(Rc<Cell<bool>>);

    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }
    let mut app = gpui::TestAppContext::single();
    let dropped = Rc::new(Cell::new(false));
    let guard = Dropped(dropped.clone());

    let host = mount(&mut app, async move {
        let _guard = guard;

        futures::future::pending().await
    });

    app.run_until_parked();
    assert!(!dropped.get());
    host.update(&mut app, |host, _, _| host.action = None)
        .unwrap();
    app.run_until_parked();
    assert!(dropped.get());
}
