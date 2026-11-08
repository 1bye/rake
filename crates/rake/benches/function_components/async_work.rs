use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use rake::component::Scope;
use rake::gpui::{self, AppContext, Context, IntoElement, Render, Window, prelude::*};
use rake::loading::{LoadStatus, Loader};
use rake::prelude::{
    Collector, HttpClient, HttpConfig, RouterConfig, Runtime, RuntimeContext, ServiceRegistry,
    Services,
};
use serde_json::json;

use super::{ALLOCATIONS, Variant, draw, percentiles};

#[derive(Default)]
struct Lifetimes {
    effects: Cell<usize>,
    futures: Cell<usize>,
    completed: Cell<usize>,
}

struct Active {
    counters: Rc<Lifetimes>,
    effect: bool,
}

impl Active {
    fn new(counters: Rc<Lifetimes>, effect: bool) -> Self {
        let count = if effect {
            &counters.effects
        } else {
            &counters.futures
        };
        count.set(count.get() + 1);
        Self { counters, effect }
    }
}

impl Drop for Active {
    fn drop(&mut self) {
        let count = if self.effect {
            &self.counters.effects
        } else {
            &self.counters.futures
        };
        count.set(count.get() - 1);
    }
}

async fn load(input: i32, pending: bool, counters: Rc<Lifetimes>) -> Result<i32, std::io::Error> {
    let _active = Active::new(counters.clone(), false);
    if pending {
        futures::future::pending::<()>().await;
    }
    counters.completed.set(counters.completed.get() + 1);
    Ok(input)
}

#[rake::component]
fn HookAsync(
    cx: &mut Scope,
    runtime: RuntimeContext,
    pending: bool,
    counters: Rc<Lifetimes>,
) -> impl IntoElement {
    let (input, setter) = cx.use_state(|| 0_i32);
    let loading = counters.clone();
    let resource = cx.use_resource(&runtime, input, move |input, _| {
        load(input, pending, loading.clone())
    });
    cx.use_effect((), move |cx| {
        let active = Active::new(counters, true);
        let task = cx.spawn(async move |cx| {
            let _ = setter.set(cx, 1);
        });
        move || drop((task, active))
    });
    let value = resource
        .read(cx, |state| match state {
            LoadStatus::Ready(value) => *value,
            _ => -1,
        })
        .unwrap();
    gpui::div().child(value.to_string())
}

struct PlainAsync {
    runtime: RuntimeContext,
    loader: Option<Loader<i32, std::io::Error>>,
    input: i32,
    loaded: Option<i32>,
    pending: bool,
    started: bool,
    counters: Rc<Lifetimes>,
    effect: Option<(gpui::Task<()>, Active)>,
}

impl PlainAsync {
    fn new(runtime: RuntimeContext, pending: bool, counters: Rc<Lifetimes>) -> Self {
        Self {
            runtime,
            loader: None,
            input: 0,
            loaded: None,
            pending,
            started: false,
            counters,
            effect: None,
        }
    }
}

impl Render for PlainAsync {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.loaded != Some(self.input) {
            self.loader.take();
            let input = self.input;
            let pending = self.pending;
            let counters = self.counters.clone();
            self.loader = Some(
                self.runtime
                    .load(cx, move |_| load(input, pending, counters.clone())),
            );
            self.loaded = Some(input);
        }

        let value = match self.loader.as_ref().unwrap().state(cx) {
            LoadStatus::Ready(value) => *value,
            _ => -1,
        };
        let first = !std::mem::replace(&mut self.started, true);
        let owner = cx.weak_entity();
        gpui::div().child(value.to_string()).child(gpui::canvas(
            |_, _, _| (),
            move |_, _, _, cx| {
                if first {
                    cx.defer(move |cx| {
                        let _ = owner.update(cx, |view, cx| {
                            let active = Active::new(view.counters.clone(), true);
                            let task = cx.spawn(async move |owner, cx| {
                                let _ = owner.update(cx, |view, cx| {
                                    view.input = 1;
                                    cx.notify();
                                });
                            });
                            view.effect = Some((task, active));
                        });
                    });
                }
            },
        ))
    }
}

#[derive(gpui::IntoElement)]
struct RawAsync {
    id: usize,
    runtime: RuntimeContext,
    pending: bool,
    counters: Rc<Lifetimes>,
}

impl gpui::RenderOnce for RawAsync {
    fn render(self, window: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let owner = window.use_keyed_state(("async", self.id), cx, |_, _| {
            PlainAsync::new(self.runtime, self.pending, self.counters)
        });
        owner.update(cx, |view, cx| view.render(window, cx).into_any_element())
    }
}

struct Host {
    variant: Variant,
    runtime: RuntimeContext,
    _driver: gpui::Task<()>,
    counters: Rc<Lifetimes>,
    plain: Vec<gpui::Entity<PlainAsync>>,
    visible: bool,
    pending: bool,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut parent = gpui::div();
        if !self.visible {
            self.plain.clear();
            return parent;
        }

        for id in 0..100 {
            parent = match self.variant {
                Variant::Plain => {
                    if id == self.plain.len() {
                        self.plain.push(cx.new(|_| {
                            PlainAsync::new(
                                self.runtime.clone(),
                                self.pending,
                                self.counters.clone(),
                            )
                        }));
                    }
                    parent.child(self.plain[id].clone())
                }
                Variant::Hooks => parent.child(HookAsync {
                    id: ("async", id),
                    runtime: self.runtime.clone(),
                    pending: self.pending,
                    counters: self.counters.clone(),
                }),
                Variant::Raw => parent.child(RawAsync {
                    id,
                    runtime: self.runtime.clone(),
                    pending: self.pending,
                    counters: self.counters.clone(),
                }),
            };
        }

        parent
    }
}

pub(super) fn measure(variant: Variant, pending: bool, samples: usize) {
    let mut cx = gpui::TestAppContext::single();
    let counters = Rc::new(Lifetimes::default());
    let window = cx.add_window(|_, cx| {
        let runtime = Runtime::new(
            RouterConfig::default(),
            Services::new(
                HttpClient::new(HttpConfig::new()).unwrap(),
                Collector::default(),
                ServiceRegistry::new(),
            ),
        );
        let context = runtime.context();
        let driver = cx.spawn(async move |_, _| {
            let _ = runtime.run().await;
        });
        Host {
            variant,
            runtime: context,
            _driver: driver,
            counters: counters.clone(),
            plain: Vec::new(),
            visible: false,
            pending,
        }
    });
    let mut times = Vec::new();
    let mut frames = Vec::new();
    let mut allocations = 0;

    for iteration in 0..samples + 3 {
        let start = Instant::now();
        let before = ALLOCATIONS.load(Ordering::Relaxed);
        window
            .update(&mut cx, |host, _, cx| {
                host.visible = true;
                cx.notify();
            })
            .unwrap();

        for _ in 0..3 {
            let start = Instant::now();
            draw(&mut cx, window);
            if iteration >= 3 {
                frames.push(start.elapsed().as_secs_f64() * 1e6);
            }
            cx.run_until_parked();
        }

        assert_eq!(counters.effects.get(), 100);
        assert_eq!(counters.futures.get(), if pending { 100 } else { 0 });

        window
            .update(&mut cx, |host, _, cx| {
                host.visible = false;
                cx.notify();
            })
            .unwrap();
        for _ in 0..2 {
            draw(&mut cx, window);
        }
        cx.run_until_parked();
        assert_eq!(counters.effects.get(), 0);
        assert_eq!(counters.futures.get(), 0);

        if iteration >= 3 {
            times.push(start.elapsed().as_secs_f64() * 1e6);
            allocations += ALLOCATIONS.load(Ordering::Relaxed) - before;
        }
    }

    let (cycle_p50, cycle_p95) = percentiles(&mut times);
    let (frame_p50, frame_p95) = percentiles(&mut frames);
    println!(
        "{}",
        json!({
            "backend": "TestAppContext", "variant": format!("{variant:?}"),
            "scenario": if pending { "effect_resource_cancel" } else { "effect_resource_async" },
            "instances": 100, "samples": samples,
            "p50_us": frame_p50, "p95_us": frame_p95,
            "p50_cycle_us": cycle_p50, "p95_cycle_us": cycle_p95,
            "allocations_per_cycle": allocations / samples,
            "live_effects_after_unmount": counters.effects.get(), "live_futures_after_unmount": counters.futures.get(),
            "completed_loads": counters.completed.get()
        })
    );
}
