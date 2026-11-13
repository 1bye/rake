use std::sync::atomic::Ordering;
use std::time::Instant;

use rake::gpui::{self, AppContext, Context, IntoElement, Render, Window, prelude::*};
use rake::reactive::{Children, Memo, Setup, Signal};
use serde_json::json;

use super::{ALLOCATIONS, LIVE_BYTES, RENDERER, Variant, draw, percentiles};

fn content(value: i64, child: impl IntoElement) -> impl IntoElement {
    gpui::div()
        .w(gpui::px(48.))
        .h(gpui::px(16.))
        .child(value.to_string())
        .child(child)
}

fn nested() -> gpui::Div {
    gpui::div().child(gpui::div().child("child"))
}

#[rake::component(cached = false)]
fn ReactiveReader(
    source: Signal<i64>,
    depth: usize,
    children: Children,
    cx: &mut Setup,
) -> impl IntoElement {
    let mut derived: Option<Memo<i64>> = None;
    for _ in 0..depth {
        let previous = derived;
        derived =
            Some(cx.memo(move |cx| {
                previous.map_or_else(|| source.get(cx), |value| value.get(cx)) + 1
            }));
    }
    content(
        derived.map_or_else(|| source.get(cx), |value| value.get(cx)),
        children,
    )
}

struct PlainReader {
    source: gpui::Entity<i64>,
    depth: usize,
    children: bool,
}

impl Render for PlainReader {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let child = if self.children {
            nested().into_any_element()
        } else {
            gpui::Empty.into_any_element()
        };
        content(*self.source.read(cx) + self.depth as i64, child)
    }
}

#[derive(gpui::IntoElement)]
struct RawReader {
    id: usize,
    source: gpui::Entity<i64>,
    depth: usize,
    children: bool,
}

impl gpui::RenderOnce for RawReader {
    fn render(self, window: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        window.use_keyed_state(("reader", self.id), cx, |_, _| PlainReader {
            source: self.source,
            depth: self.depth,
            children: self.children,
        })
    }
}

struct Host {
    variant: Variant,
    source: gpui::Entity<i64>,
    signal: Signal<i64>,
    count: usize,
    depth: usize,
    children: bool,
    plain: Vec<gpui::Entity<PlainReader>>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.count == 0 {
            self.plain.clear();
        }
        let mut parent = gpui::div().flex().flex_wrap().w(gpui::px(960.));
        for id in 0..self.count {
            parent = match self.variant {
                Variant::Plain => {
                    if self.plain.len() == id {
                        self.plain.push(cx.new(|_| PlainReader {
                            source: self.source.clone(),
                            depth: self.depth,
                            children: self.children,
                        }));
                    }
                    parent.child(self.plain[id].clone())
                }
                Variant::Signals => {
                    let children = if self.children {
                        Children::new(|_, _| nested().into_any_element())
                    } else {
                        Children::default()
                    };
                    parent.child(
                        ReactiveReader::builder()
                            .id(("reader", id))
                            .source(self.signal)
                            .depth(self.depth)
                            .children(children)
                            .build(),
                    )
                }
                Variant::Raw => parent.child(RawReader {
                    id,
                    source: self.source.clone(),
                    depth: self.depth,
                    children: self.children,
                }),
                _ => unreachable!("dependency cases use uncached variants"),
            };
        }
        parent
    }
}

fn measure(variant: Variant, count: usize, depth: usize, children: bool, samples: usize) {
    let mut cx = gpui::TestAppContext::single();
    let (source, signal) = cx.update(|cx| {
        let source = cx.new(|_| 0_i64);
        let signal = Signal::from_entity(cx, source.clone());
        (source, signal)
    });
    let baseline = cx.update(|cx| rake::reactive::runtime_stats(cx));
    let bytes = LIVE_BYTES.load(Ordering::Relaxed);
    let window = cx.add_window(|_, _| Host {
        variant,
        source: source.clone(),
        signal,
        count,
        depth,
        children,
        plain: Vec::new(),
    });
    draw(&mut cx, window);
    let after_mount = LIVE_BYTES.load(Ordering::Relaxed) - bytes;
    let mut times = Vec::with_capacity(samples);
    let mut allocations = 0;
    for iteration in 0..samples + 5 {
        let before = ALLOCATIONS.load(Ordering::Relaxed);
        let started = Instant::now();
        cx.update(|cx| {
            if matches!(variant, Variant::Signals) {
                signal.set(cx, iteration as i64);
            } else {
                source.update(cx, |value, cx| {
                    *value = iteration as i64;
                    cx.notify();
                });
            }
        });
        draw(&mut cx, window);
        let elapsed = started.elapsed().as_secs_f64() * 1e6;
        if iteration >= 5 {
            times.push(elapsed);
            allocations += ALLOCATIONS.load(Ordering::Relaxed) - before;
        }
        cx.run_until_parked();
    }
    window
        .update(&mut cx, |host, _, cx| {
            host.count = 0;
            cx.notify();
        })
        .unwrap();
    draw(&mut cx, window);
    draw(&mut cx, window);
    cx.run_until_parked();
    let after = cx.update(|cx| rake::reactive::runtime_stats(cx));
    assert_eq!(after, baseline);
    cx.update(|cx| signal.dispose(cx));
    let (p50, p95) = percentiles(&mut times);
    println!(
        "{}",
        json!({
            "renderer": RENDERER, "backend": "TestAppContext", "variant": format!("{variant:?}"),
            "scenario": if children { "children" } else if depth > 0 { "memo_chain" } else { "fanout" },
            "instances": count, "depth": depth, "samples": samples, "p50_us": p50, "p95_us": p95,
            "allocations_per_sample": allocations as f64 / samples as f64, "live_after_mount_bytes": after_mount,
            "runtime_after_unmount": after, "runtime_baseline": baseline
        })
    );
}

pub(super) fn run(samples: usize) {
    for variant in [Variant::Plain, Variant::Signals, Variant::Raw] {
        for count in [1, 100, 1_000] {
            measure(variant, count, 0, false, samples);
        }
        for depth in [1, 10, 50] {
            measure(variant, 1, depth, false, samples);
        }
        measure(variant, 1, 0, true, samples);
    }
}
