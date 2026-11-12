use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use futures::channel::oneshot;
use rake::gpui::{self, App, AppContext, Context, IntoElement, Render, Window, prelude::*};
use rake::loading::{LoadState, LoadStatus};
use rake::reactive::{Resource, Setup, Signal};
use serde_json::json;

use super::{ALLOCATIONS, RENDERER, Variant, draw, percentiles};

type Reply = oneshot::Sender<Result<i32, std::io::Error>>;

#[derive(Default)]
struct Requests {
    replies: RefCell<Vec<(i32, Reply)>>,
    signal: Cell<Option<Signal<i32>>>,
    resource: Cell<Option<Resource<i32, std::io::Error>>>,
    plain: RefCell<Option<gpui::WeakEntity<Plain>>>,
    value: Cell<Option<i32>>,
}

impl Requests {
    fn load(&self, input: i32) -> impl Future<Output = Result<i32, std::io::Error>> + use<> {
        let (send, receive) = oneshot::channel();
        self.replies.borrow_mut().push((input, send));
        async move { receive.await.map_err(std::io::Error::other)? }
    }

    fn update(&self, input: i32, retry: bool, cx: &mut App) {
        if let Some(signal) = self.signal.get() {
            signal.set(cx, input);
            if retry {
                self.resource.get().unwrap().retry(cx);
            }
        } else {
            self.plain
                .borrow()
                .as_ref()
                .unwrap()
                .update(cx, |view, cx| {
                    view.input = input;
                    if retry {
                        view.loaded = None;
                    }
                    cx.notify();
                })
                .unwrap();
        }
    }

    fn complete(&self, expected: i32) {
        let (input, latest) = self.replies.borrow_mut().pop().unwrap();
        assert_eq!(input, expected);
        for (_, cancelled) in self.replies.borrow_mut().drain(..) {
            assert!(cancelled.send(Ok(-1)).is_err());
        }
        assert!(latest.send(Ok(expected)).is_ok());
    }
}

#[rake::component(cached = false)]
fn ReactiveRequest(requests: Rc<Requests>, cx: &mut Setup) -> impl IntoElement {
    let input = cx.signal(0);
    let pending = requests.clone();
    let resource = cx.resource(move |cx| pending.load(input.get(cx)));
    requests.signal.set(Some(input));
    requests.resource.set(Some(resource));
    {
        requests.value.set(resource.latest(cx).copied());
        gpui::div().child(format!("{:?}", requests.value.get()))
    }
}

struct Plain {
    input: i32,
    loaded: Option<i32>,
    load: LoadState<i32, std::io::Error>,
    latest: Option<i32>,
    task: Option<gpui::Task<()>>,
    requests: Rc<Requests>,
}

impl Plain {
    fn new(requests: Rc<Requests>, cx: &mut Context<Self>) -> Self {
        *requests.plain.borrow_mut() = Some(cx.weak_entity());
        Self {
            input: 0,
            loaded: None,
            load: LoadState::new(),
            latest: None,
            task: None,
            requests,
        }
    }
}

impl Render for Plain {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.loaded != Some(self.input) {
            self.task.take();
            if let Some(value) = self.load.take_ready() {
                self.latest = Some(value);
            }
            let token = self.load.begin();
            let future = self.requests.load(self.input);
            self.task = Some(cx.spawn(async move |view, cx| {
                let result = future.await;
                let _ = view.update(cx, |view, cx| {
                    if view.load.complete(token, result) {
                        cx.notify();
                    }
                });
            }));
            self.loaded = Some(self.input);
        }
        let value = match self.load.state() {
            LoadStatus::Ready(value) => Some(*value),
            _ => self.latest,
        };
        self.requests.value.set(value);
        gpui::div().child(format!("{value:?}"))
    }
}

#[derive(gpui::IntoElement)]
struct RawRequest(Rc<Requests>);

impl gpui::RenderOnce for RawRequest {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        window.use_keyed_state("request", cx, |_, cx| Plain::new(self.0, cx))
    }
}

struct Host {
    variant: Variant,
    requests: Rc<Requests>,
    plain: Option<gpui::Entity<Plain>>,
    visible: bool,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.visible {
            self.plain.take();
            return gpui::div();
        }
        let child = match self.variant {
            Variant::Plain => self
                .plain
                .get_or_insert_with(|| cx.new(|cx| Plain::new(self.requests.clone(), cx)))
                .clone()
                .into_any_element(),
            Variant::Signals => ReactiveRequest::builder()
                .id("request")
                .requests(self.requests.clone())
                .build()
                .into_any_element(),
            Variant::Raw => RawRequest(self.requests.clone()).into_any_element(),
            _ => unreachable!("resource cases use uncached variants"),
        };
        gpui::div().child(child)
    }
}

fn measure(variant: Variant, writes: usize, retry: bool, samples: usize) {
    let mut cx = gpui::TestAppContext::single();
    let requests = Rc::new(Requests::default());
    let window = cx.add_window(|_, _| Host {
        variant,
        requests: requests.clone(),
        plain: None,
        visible: true,
    });
    draw(&mut cx, window);
    cx.run_until_parked();
    let mut times = Vec::with_capacity(samples);
    let mut allocations = 0;
    for iteration in 0..samples + 5 {
        if retry {
            cx.update(|cx| requests.update((iteration + 1) as i32, false, cx));
            draw(&mut cx, window);
            cx.run_until_parked();
        }
        let before = ALLOCATIONS.load(Ordering::Relaxed);
        let started = Instant::now();
        cx.update(|cx| {
            for value in 1..=writes {
                requests.update((iteration * writes + value) as i32, retry, cx);
            }
        });
        draw(&mut cx, window);
        cx.run_until_parked();
        let elapsed = started.elapsed().as_secs_f64() * 1e6;
        if iteration >= 5 {
            times.push(elapsed);
            allocations += ALLOCATIONS.load(Ordering::Relaxed) - before;
        }
        let expected = ((iteration + 1) * writes) as i32;
        requests.complete(expected);
        cx.run_until_parked();
        draw(&mut cx, window);
        assert_eq!(requests.value.get(), Some(expected));
    }
    cx.update(|cx| requests.update(-1, true, cx));
    draw(&mut cx, window);
    cx.run_until_parked();
    window
        .update(&mut cx, |host, _, cx| {
            host.visible = false;
            cx.notify();
        })
        .unwrap();
    draw(&mut cx, window);
    draw(&mut cx, window);
    cx.run_until_parked();
    for (_, cancelled) in requests.replies.borrow_mut().drain(..) {
        assert!(cancelled.send(Ok(-1)).is_err());
    }
    let stats = cx.update(|cx| rake::reactive::runtime_stats(cx));
    assert_eq!(stats, rake::reactive::RuntimeStats::default());
    let (p50, p95) = percentiles(&mut times);
    println!(
        "{}",
        json!({
            "renderer": RENDERER, "backend": "TestAppContext", "variant": format!("{variant:?}"),
            "scenario": if retry { "resource_retry" } else { "resource_burst" }, "writes": writes, "samples": samples,
            "p50_us": p50, "p95_us": p95, "allocations_per_sample": allocations as f64 / samples as f64,
            "runtime_after_unmount": stats
        })
    );
}

pub(super) fn run(samples: usize) {
    for variant in [Variant::Plain, Variant::Signals, Variant::Raw] {
        for writes in [1, 10, 100] {
            measure(variant, writes, false, samples);
        }
        measure(variant, 1, true, samples);
    }
}
