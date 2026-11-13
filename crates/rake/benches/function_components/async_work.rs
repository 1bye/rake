use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use rake::gpui::{self, AppContext, Context, IntoElement, Render, Window, prelude::*};
use rake::loading::{LoadState, LoadStatus};
use rake::reactive::{ReactiveApp, Setup};
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
fn SignalAsync(pending: bool, counters: Rc<Lifetimes>, cx: &mut Setup) -> impl IntoElement {
    let input = cx.signal(0_i32);
    let loading = counters.clone();
    let resource = cx.resource(move |cx| load(input.get(cx), pending, loading.clone()));
    cx.spawn(async move |cx| {
        input.set(cx, 1);
    });
    cx.effect(move |_, cx| {
        let active = Active::new(counters.clone(), true);
        cx.on_cleanup(move |_| drop(active));
    });
    gpui::div().child(match resource.read(cx) {
        LoadStatus::Ready(value) => value.to_string(),
        _ => "-1".to_owned(),
    })
}

struct PlainAsync {
    load: LoadState<i32, std::io::Error>,
    task: Option<gpui::Task<()>>,
    input: i32,
    loaded: Option<i32>,
    pending: bool,
    started: bool,
    counters: Rc<Lifetimes>,
    effect: Option<(gpui::Task<()>, Active)>,
}

impl PlainAsync {
    fn new(pending: bool, counters: Rc<Lifetimes>) -> Self {
        Self {
            load: LoadState::new(),
            task: None,
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
            self.task.take();
            let input = self.input;
            let pending = self.pending;
            let counters = self.counters.clone();
            let token = self.load.begin();
            self.task = Some(cx.spawn(async move |owner, cx| {
                let result = load(input, pending, counters).await;
                let _ = owner.update(cx, |view, cx| {
                    if view.load.complete(token, result) {
                        cx.notify();
                    }
                });
            }));
            self.loaded = Some(input);
        }

        let value = match self.load.state() {
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
    pending: bool,
    counters: Rc<Lifetimes>,
}

impl gpui::RenderOnce for RawAsync {
    fn render(self, window: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let owner = window.use_keyed_state(("async", self.id), cx, |_, _| {
            PlainAsync::new(self.pending, self.counters)
        });
        owner.update(cx, |view, cx| view.render(window, cx).into_any_element())
    }
}

struct Host {
    variant: Variant,
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
                Variant::Plain | Variant::PlainCached => {
                    if id == self.plain.len() {
                        self.plain
                            .push(cx.new(|_| PlainAsync::new(self.pending, self.counters.clone())));
                    }
                    parent.child(self.plain[id].clone())
                }
                Variant::Signals | Variant::Cached => parent.child(SignalAsync {
                    id: ("async", id).into(),
                    pending: self.pending,
                    counters: self.counters.clone(),
                }),
                Variant::Raw => parent.child(RawAsync {
                    id,
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
    let window = cx.add_window(|_, _| Host {
        variant,
        counters: counters.clone(),
        plain: Vec::new(),
        visible: false,
        pending,
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
            "renderer": super::RENDERER, "backend": "TestAppContext", "variant": format!("{variant:?}"),
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
