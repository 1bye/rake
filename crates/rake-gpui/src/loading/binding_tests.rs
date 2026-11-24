//! GPUI-facing contract tests for the public [`super::Loader`] binding.
#![cfg(all(feature = "view-test-support", not(target_family = "wasm")))]

use std::cell::{Cell, RefCell};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::rc::Rc;

use crate::diagnostics::Collector;
use crate::gpui::{self, Context, IntoElement, Render, Task, Window, prelude::*};
use crate::http::{HttpClient, HttpConfig};
use crate::loading::{LoadError, LoadStatus, Loader};
use crate::prelude::Runtime;
use crate::router::{NoRoute, Router, RouterConfig};
use crate::services::{ServiceRegistry, Services};

#[derive(Debug)]
struct SourceError(&'static str);

impl Display for SourceError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}

impl Error for SourceError {}

#[derive(Clone)]
struct Marker(Rc<Cell<u32>>);

struct Harness<T: 'static, E: Error + 'static> {
    loader: Option<Loader<T, E>>,
    router: Router<NoRoute>,
    _driver: Task<()>,
    renders: Rc<Cell<usize>>,
}

impl<T: 'static, E: Error + 'static> Render for Harness<T, E> {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        gpui::div()
    }
}

struct PairHarness<E: Error + 'static> {
    first: Loader<String, E>,
    second: Loader<String, E>,
    router: Router<NoRoute>,
    _driver: Task<()>,
}

impl<E: Error + 'static> Render for PairHarness<E> {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::div()
    }
}

fn services() -> Services {
    Services::new(
        HttpClient::new(HttpConfig::new()).expect("http service"),
        Collector::default(),
        ServiceRegistry::new(),
    )
}

fn harness<T, E, F, Fut>(
    cx: &mut gpui::TestAppContext,
    config: RouterConfig,
    services: Services,
    factory: F,
    renders: Rc<Cell<usize>>,
) -> gpui::WindowHandle<Harness<T, E>>
where
    T: 'static,
    E: Error + 'static,
    F: Fn(crate::loading::LoadContext) -> Fut + 'static,
    Fut: std::future::Future<Output = Result<T, E>> + 'static,
{
    cx.add_window(move |_, cx| {
        let runtime = Runtime::new(config, services);
        let context = runtime.context();
        let router = runtime.router.clone();
        let driver = cx.spawn(async move |_, _| {
            let _ = runtime.run().await;
        });
        let loader = context.load(cx, factory);

        Harness {
            loader: Some(loader),
            router,
            _driver: driver,
            renders,
        }
    })
}

fn shutdown<T: 'static, E: Error + 'static>(
    cx: &mut gpui::TestAppContext,
    window: gpui::WindowHandle<Harness<T, E>>,
) {
    window
        .read_with(cx, |harness, _| harness.router.shutdown())
        .expect("runtime alive");
    cx.run_until_parked();
}

#[test]
fn initial_load_accepts_move_only_data_and_notifies_owner() {
    struct Value(String);

    let mut cx = gpui::TestAppContext::single();
    let renders = Rc::new(Cell::new(0));
    let window = harness(
        &mut cx,
        RouterConfig::default(),
        services(),
        |_context| async { Ok::<_, SourceError>(Value("ready".into())) },
        renders.clone(),
    );

    cx.update_window(window.into(), |_, window, cx| {
        window.draw(cx).clear(cx);
    })
    .expect("initial draw");
    let before = renders.get();
    cx.run_until_parked();

    assert!(
        renders.get() > before,
        "load completion should redraw the owner"
    );
    window
        .read_with(&cx, |harness, cx| {
            assert!(matches!(
                harness.loader.as_ref().unwrap().state(cx),
                LoadStatus::Ready(value) if value.0 == "ready"
            ));
        })
        .unwrap();
    shutdown(&mut cx, window);
}

#[test]
fn same_value_type_loaders_have_independent_models() {
    let mut cx = gpui::TestAppContext::single();
    let window = cx.add_window(|_, cx| {
        let runtime = Runtime::new(RouterConfig::default(), services());
        let context = runtime.context();
        let router = runtime.router.clone();
        let driver = cx.spawn(async move |_, _| {
            let _ = runtime.run().await;
        });
        let first = context.load(cx, |_context| async {
            Ok::<_, SourceError>(String::from("first"))
        });
        let second = context.load(cx, |_context| async {
            Ok::<_, SourceError>(String::from("second"))
        });

        PairHarness {
            first,
            second,
            router,
            _driver: driver,
        }
    });

    cx.run_until_parked();
    window
        .read_with(&cx, |harness, cx| {
            assert!(
                matches!(harness.first.state(cx), LoadStatus::Ready(value) if value == "first")
            );
            assert!(
                matches!(harness.second.state(cx), LoadStatus::Ready(value) if value == "second")
            );
        })
        .unwrap();
    window
        .read_with(&cx, |harness, _| harness.router.shutdown())
        .unwrap();
    cx.run_until_parked();
}

#[test]
fn retry_cancels_old_attempt_and_only_new_result_wins() {
    let mut cx = gpui::TestAppContext::single();
    let calls = Rc::new(Cell::new(0));
    let observed = calls.clone();
    let window = harness(
        &mut cx,
        RouterConfig::default(),
        services(),
        move |_context| {
            let call = observed.get();
            observed.set(call + 1);
            async move {
                if call == 0 {
                    futures::future::pending::<Result<String, SourceError>>().await
                } else {
                    Ok("retry".into())
                }
            }
        },
        Rc::new(Cell::new(0)),
    );

    cx.run_until_parked();
    window
        .update(&mut cx, |harness, _, cx| {
            harness.loader.as_mut().unwrap().retry(cx);
        })
        .unwrap();
    cx.run_until_parked();

    window
        .read_with(&cx, |harness, cx| {
            assert!(matches!(
                harness.loader.as_ref().unwrap().state(cx),
                LoadStatus::Ready(value) if value == "retry"
            ));
        })
        .unwrap();
    shutdown(&mut cx, window);
}

#[test]
fn dropping_loader_cancels_its_managed_task() {
    let mut cx = gpui::TestAppContext::single();
    let window = harness(
        &mut cx,
        RouterConfig::default(),
        services(),
        |_context| async { futures::future::pending::<Result<String, SourceError>>().await },
        Rc::new(Cell::new(0)),
    );

    cx.run_until_parked();
    window
        .update(&mut cx, |harness, _, _| {
            harness.loader.take();
        })
        .unwrap();
    cx.run_until_parked();

    window
        .read_with(&cx, |harness, _| {
            assert!(harness.router.snapshot().work.tasks.iter().all(|task| {
                task.key != "load" || matches!(task.state, crate::tasks::TaskState::Cancelled)
            }));
        })
        .unwrap();
    shutdown(&mut cx, window);
}

#[test]
fn moved_loader_reports_owner_release_even_while_the_handle_is_retained() {
    let mut cx = gpui::TestAppContext::single();
    let retained = Rc::new(RefCell::new(None));
    let retained_in_window = retained.clone();
    let window = harness(
        &mut cx,
        RouterConfig::default(),
        services(),
        |_context| async { futures::future::pending::<Result<String, SourceError>>().await },
        Rc::new(Cell::new(0)),
    );
    let router = window
        .read_with(&cx, |harness, _| harness.router.clone())
        .unwrap();

    window
        .update(&mut cx, |harness, _, _| {
            *retained_in_window.borrow_mut() = harness.loader.take();
        })
        .unwrap();
    cx.update_window(window.into(), |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();

    let retained = retained.borrow();
    let loader = retained.as_ref().expect("loader moved out of owner");
    cx.read(|cx| {
        assert!(matches!(
            loader.state(cx),
            LoadStatus::Failed(LoadError::Task(crate::tasks::TaskError::OwnerDisposed))
        ))
    });
    router.shutdown();
    cx.run_until_parked();
}

#[test]
fn zero_task_capacity_is_a_visible_typed_failure() {
    let mut cx = gpui::TestAppContext::single();
    let window = harness(
        &mut cx,
        RouterConfig {
            tasks: crate::tasks::TaskConfig {
                capacity: 0,
                ..Default::default()
            },
            ..Default::default()
        },
        services(),
        |_context| async { Ok::<_, SourceError>(String::from("never")) },
        Rc::new(Cell::new(0)),
    );

    cx.run_until_parked();
    window
        .read_with(&cx, |harness, cx| {
            assert!(matches!(
                harness.loader.as_ref().unwrap().state(cx),
                LoadStatus::Failed(LoadError::Task(crate::tasks::TaskError::Capacity))
            ));
        })
        .unwrap();
    shutdown(&mut cx, window);
}

#[test]
fn source_errors_remain_typed_and_load_context_preserves_services() {
    let mut cx = gpui::TestAppContext::single();
    let marker = Rc::new(Cell::new(0));
    let registry = ServiceRegistry::new()
        .with(Marker(marker.clone()))
        .expect("marker service");
    let configured_http = HttpClient::new(HttpConfig::new()).expect("http");
    let services = Services::new(configured_http.clone(), Collector::default(), registry);
    let seen = marker.clone();
    let expected_http = configured_http.clone();
    let window = harness(
        &mut cx,
        RouterConfig::default(),
        services,
        move |context| {
            let seen = seen.clone();
            let expected_http = expected_http.clone();
            async move {
                let service = context.services().get::<Marker>().expect("marker");
                assert!(Rc::ptr_eq(&service.0, &seen));
                assert!(context.http().shares_transport_with(&expected_http));
                assert!(!context.cancellation().is_cancelled());
                let _background = context.background_executor();
                seen.set(1);
                Err::<String, _>(SourceError("offline"))
            }
        },
        Rc::new(Cell::new(0)),
    );

    cx.run_until_parked();
    window
        .read_with(&cx, |harness, cx| {
            assert!(matches!(
                harness.loader.as_ref().unwrap().state(cx),
                LoadStatus::Failed(LoadError::Source(error)) if error.0 == "offline"
            ));
        })
        .unwrap();
    assert_eq!(marker.get(), 1);
    shutdown(&mut cx, window);
}

#[test]
fn explicit_cancel_is_a_terminal_cancelled_error() {
    let mut cx = gpui::TestAppContext::single();
    let window = harness(
        &mut cx,
        RouterConfig::default(),
        services(),
        |_context| async { futures::future::pending::<Result<String, SourceError>>().await },
        Rc::new(Cell::new(0)),
    );

    cx.run_until_parked();
    window
        .update(&mut cx, |harness, _, cx| {
            harness.loader.as_mut().unwrap().cancel(cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .read_with(&cx, |harness, cx| {
            assert!(matches!(
                harness.loader.as_ref().unwrap().state(cx),
                LoadStatus::Failed(LoadError::Cancelled)
            ));
        })
        .unwrap();
    shutdown(&mut cx, window);
}

#[test]
fn runtime_shutdown_converts_pending_load_to_terminal_failure() {
    let mut cx = gpui::TestAppContext::single();
    let window = harness(
        &mut cx,
        RouterConfig::default(),
        services(),
        |_context| async { futures::future::pending::<Result<String, SourceError>>().await },
        Rc::new(Cell::new(0)),
    );

    cx.run_until_parked();
    window
        .read_with(&cx, |harness, _| harness.router.shutdown())
        .unwrap();
    cx.run_until_parked();
    window
        .read_with(&cx, |harness, cx| {
            assert!(matches!(
                harness.loader.as_ref().unwrap().state(cx),
                LoadStatus::Failed(LoadError::Cancelled | LoadError::Task(_))
            ));
        })
        .unwrap();
}
