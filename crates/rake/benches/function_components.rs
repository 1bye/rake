#![doc = "Equivalent GPUI, function-component, and raw keyed-state workloads. Supports headless measurements and an actual native window."]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
use std::time::Instant;

use rake::gpui::{self, AppContext, Context, IntoElement, Render, Window, prelude::*};
use rake::reactive::{Setup, Signal};
use serde_json::json;

#[cfg(feature = "gpui-fast")]
use gpui_fast_platform as gpui_platform;

#[path = "function_components/async_work.rs"]
mod async_work;
#[path = "function_components/dependencies.rs"]
mod dependencies;
#[path = "function_components/resources.rs"]
mod resources;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static LIVE_BYTES: AtomicIsize = AtomicIsize::new(0);
const RENDERER: &str = if cfg!(feature = "gpui-fast") {
    "gpui-fast 0.1.13"
} else {
    "gpui-pre 0.3.11"
};
const SCENARIOS: [&str; 6] = ["mount", "parent", "leaf", "all", "reorder", "cycle"];

struct CountingAllocator;

#[expect(
    clippy::undocumented_unsafe_blocks,
    reason = "The wrapper forwards each original pointer and layout unchanged to System; counters never access allocated memory."
)]
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };

        if !pointer.is_null() {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            LIVE_BYTES.fetch_add(layout.size() as isize, Ordering::Relaxed);
        }

        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        LIVE_BYTES.fetch_sub(layout.size() as isize, Ordering::Relaxed);
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let pointer = unsafe { System.realloc(pointer, layout, size) };

        if !pointer.is_null() {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            LIVE_BYTES.fetch_add(size as isize - layout.size() as isize, Ordering::Relaxed);
        }

        pointer
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[derive(Clone, Copy, Debug)]
enum Variant {
    Plain,
    PlainCached,
    Signals,
    Cached,
    Raw,
}

#[derive(Default)]
enum Writer {
    #[default]
    Empty,
    Plain(gpui::WeakEntity<Plain>),
    Signals(Signal<i32>),
    Raw(gpui::WeakEntity<i32>),
}

impl Writer {
    fn increment(&self, cx: &mut gpui::App) -> bool {
        match self {
            Self::Empty => false,
            Self::Plain(owner) => owner
                .update(cx, |row, cx| {
                    row.values[0] += 1;
                    cx.notify();
                })
                .is_ok(),
            Self::Signals(state) => state.update(cx, |value| *value += 1).is_some(),
            Self::Raw(owner) => owner
                .update(cx, |value, cx| {
                    *value += 1;
                    cx.notify();
                })
                .is_ok(),
        }
    }
}

#[derive(Default)]
struct Probe {
    writer: RefCell<Writer>,
    rendered: Cell<i32>,
}

impl PartialEq for Probe {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

fn row(probe: &Probe, value: i32) -> impl IntoElement + use<> {
    probe.rendered.set(value);
    gpui::div()
        .w(gpui::px(48.))
        .h(gpui::px(16.))
        .child(value.to_string())
}

struct Plain {
    values: Vec<i32>,
    probe: Rc<Probe>,
}

impl Render for Plain {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.probe.writer.replace(Writer::Plain(cx.weak_entity()));
        row(&self.probe, self.values.iter().sum())
    }
}

#[rake::component(cached = false)]
fn SignalRow(signals: usize, probe: Rc<Probe>, cx: &mut Setup) -> impl IntoElement {
    let values = create_signals(cx, signals, &probe);
    row(&probe, values.iter().map(|value| value.get(cx)).sum())
}

#[rake::component(cached = "w-12 h-4")]
fn CachedRow(signals: usize, probe: Rc<Probe>, cx: &mut Setup) -> impl IntoElement {
    let values = create_signals(cx, signals, &probe);
    row(&probe, values.iter().map(|value| value.get(cx)).sum())
}

fn create_signals<V: 'static>(
    cx: &mut Setup<'_, '_, V>,
    count: usize,
    probe: &Probe,
) -> Vec<Signal<i32>> {
    let values: Vec<_> = (0..count).map(|_| cx.signal(0)).collect();
    probe.writer.replace(Writer::Signals(values[0]));
    values
}

#[derive(gpui::IntoElement)]
struct RawRow {
    id: usize,
    signals: usize,
    probe: Rc<Probe>,
}

impl gpui::RenderOnce for RawRow {
    fn render(self, window: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let mut sum = 0;
        window.with_id(("row", self.id), |window| {
            for index in 0..self.signals {
                let state = window.use_keyed_state(("hook", index), cx, |_, _| 0_i32);
                sum += *state.read(cx);

                if index == 0 {
                    self.probe.writer.replace(Writer::Raw(state.downgrade()));
                }
            }
        });
        row(&self.probe, sum)
    }
}

struct Row {
    id: usize,
    probe: Rc<Probe>,
    plain: Option<gpui::Entity<Plain>>,
}

struct Host {
    variant: Variant,
    signals: usize,
    rows: Vec<Row>,
    next: usize,
}

impl Host {
    fn new(variant: Variant, signals: usize) -> Self {
        Self {
            variant,
            signals,
            rows: Vec::new(),
            next: 0,
        }
    }

    fn append(&mut self) {
        self.rows.push(Row {
            id: self.next,
            probe: Rc::new(Probe::default()),
            plain: None,
        });
        self.next += 1;
    }

    fn change(&mut self, scenario: &str, count: usize, cx: &mut Context<Self>) {
        match scenario {
            "mount" | "cycle" => {
                self.rows.clear();
                for _ in 0..count {
                    self.append();
                }
            }
            "reorder" => {
                self.rows.reverse();
                self.rows.pop();
                self.append();
            }
            "parent" | "signals" => {}
            _ => panic!("unknown scenario"),
        }

        cx.notify();
    }
}

fn change_values(scenario: &str, probes: &[Rc<Probe>], cx: &mut gpui::App) -> bool {
    let count = match scenario {
        "leaf" => 1,
        "all" => probes.len(),
        _ => return false,
    };
    for probe in &probes[..count] {
        assert!(probe.writer.borrow().increment(cx));
    }
    true
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut parent = gpui::div().flex().flex_wrap().w(gpui::px(960.));

        for item in &mut self.rows {
            parent = match self.variant {
                Variant::Plain | Variant::PlainCached => {
                    let view = item.plain.get_or_insert_with(|| {
                        cx.new(|_| Plain {
                            values: vec![0; self.signals],
                            probe: item.probe.clone(),
                        })
                    });
                    if matches!(self.variant, Variant::PlainCached) {
                        view.read(cx);
                        parent.child(
                            view.clone().cached(
                                gpui::StyleRefinement::default()
                                    .w(gpui::px(48.))
                                    .h(gpui::px(16.)),
                            ),
                        )
                    } else {
                        parent.child(view.clone())
                    }
                }
                Variant::Signals => parent.child(SignalRow {
                    id: ("row", item.id).into(),
                    signals: self.signals,
                    probe: item.probe.clone(),
                }),
                Variant::Cached => parent.child(CachedRow {
                    id: ("row", item.id).into(),
                    signals: self.signals,
                    probe: item.probe.clone(),
                }),
                Variant::Raw => parent.child(RawRow {
                    id: item.id,
                    signals: self.signals,
                    probe: item.probe.clone(),
                }),
            };
        }

        parent
    }
}

fn draw<V: Render>(cx: &mut gpui::TestAppContext, window: gpui::WindowHandle<V>) {
    cx.update_window(window.into(), |_, window, cx| {
        window.draw(cx).clear(cx);
        window.simulate_next_frame(cx);
    })
    .unwrap();
}

fn percentiles(times: &mut [f64]) -> (f64, f64) {
    times.sort_by(f64::total_cmp);
    (times[times.len() / 2], times[times.len() * 95 / 100])
}

fn measure(variant: Variant, count: usize, signals: usize, scenario: &str, samples: usize) {
    let mut cx = gpui::TestAppContext::single();
    let window = cx.add_window(|_, _| Host::new(variant, signals));
    cx.run_until_parked();
    draw(&mut cx, window);
    let baseline = LIVE_BYTES.load(Ordering::Relaxed);
    window
        .update(&mut cx, |host, _, cx| host.change("mount", count, cx))
        .unwrap();
    cx.run_until_parked();
    draw(&mut cx, window);
    let after_mount = LIVE_BYTES.load(Ordering::Relaxed) - baseline;
    let probes: Vec<_> = window
        .read_with(&cx, |host, _| {
            host.rows.iter().map(|row| row.probe.clone()).collect()
        })
        .unwrap();
    let mut times = Vec::with_capacity(samples);
    let mut allocations = 0;

    for iteration in 0..samples + 5 {
        let before = ALLOCATIONS.load(Ordering::Relaxed);
        let start = Instant::now();
        if !cx.update(|cx| change_values(scenario, &probes, cx)) {
            window
                .update(&mut cx, |host, _, cx| host.change(scenario, count, cx))
                .unwrap();
        }
        draw(&mut cx, window);
        let elapsed = start.elapsed().as_secs_f64() * 1e6;
        let allocated = ALLOCATIONS.load(Ordering::Relaxed) - before;
        if matches!(scenario, "leaf" | "all") {
            assert_eq!(
                probes[0].rendered.get(),
                (iteration + 1) as i32,
                "updated row must render its current value"
            );
        }
        cx.run_until_parked();

        if iteration >= 5 {
            times.push(elapsed);
            allocations += allocated;
        }
    }

    window
        .update(&mut cx, |host, _, cx| {
            host.rows.clear();
            cx.notify();
        })
        .unwrap();
    draw(&mut cx, window);
    draw(&mut cx, window);
    cx.run_until_parked();
    let disposed = cx.update(|cx| {
        probes
            .iter()
            .all(|probe| !probe.writer.borrow().increment(cx))
    });
    drop(probes);
    let retained = LIVE_BYTES.load(Ordering::Relaxed) - baseline;
    let runtime_stats = cx.update(|cx| rake::reactive::runtime_stats(cx));
    assert_eq!(runtime_stats, rake::reactive::RuntimeStats::default());
    let (p50, p95) = percentiles(&mut times);
    println!(
        "{}",
        json!({
            "renderer": RENDERER, "backend": "TestAppContext", "variant": format!("{variant:?}"), "scenario": scenario,
            "instances": count, "signals": signals, "samples": samples,
            "p50_us": p50, "p95_us": p95,
            "allocations_per_sample": allocations as f64 / samples as f64,
            "live_after_mount_bytes": after_mount, "retained_after_unmount_bytes": retained,
            "initial_handles_disposed": disposed, "runtime_after_unmount": runtime_stats
        })
    );
    assert!(disposed, "component retained after unmount");
}

struct Native {
    host: gpui::Entity<Host>,
    samples: Rc<RefCell<Vec<f64>>>,
    scenario: &'static str,
}

impl Render for Native {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = self.host.downgrade();
        let host = self.host.read(cx);
        let samples = self.samples.clone();
        let variant = host.variant;
        let scenario = self.scenario;
        let instances = host.rows.len();
        let signals = host.signals;
        let probes: Vec<_> = host.rows.iter().map(|row| row.probe.clone()).collect();
        let started = Instant::now();
        let measurement = gpui::canvas(
            |_, _, _| (),
            move |_, _, window, cx| {
                if samples.borrow().len() >= 65 {
                    return;
                }

                samples
                    .borrow_mut()
                    .push(started.elapsed().as_secs_f64() * 1e6);

                if samples.borrow().len() >= 65 {
                    let mut times = samples.borrow()[5..].to_vec();
                    let (p50, p95) = percentiles(&mut times);
                    println!(
                        "{}",
                        json!({
                            "renderer": RENDERER, "backend": "native", "variant": format!("{variant:?}"),
                            "scenario": scenario, "instances": instances, "signals": signals,
                            "samples": times.len(), "p50_us": p50, "p95_us": p95
                        })
                    );
                    cx.quit();
                } else {
                    window.on_next_frame(move |_, cx| {
                        if !change_values(scenario, &probes, cx) {
                            let _ = owner.update(cx, |host, cx| host.change(scenario, instances, cx));
                        }
                    });
                    window.request_animation_frame();
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size(gpui::px(1.));

        gpui::div()
            .size_full()
            .child(self.host.clone())
            .child(measurement)
    }
}

fn main() {
    let args: Vec<_> = std::env::args().filter(|arg| arg != "--bench").collect();

    if let Some(index) = args
        .iter()
        .position(|arg| arg == "--native" || arg == "--headless")
    {
        let variant = match args.get(index + 1).map(String::as_str) {
            Some("plain") => Variant::Plain,
            Some("plain-cached") => Variant::PlainCached,
            Some("signals") => Variant::Signals,
            Some("cached") => Variant::Cached,
            Some("raw") => Variant::Raw,
            _ => panic!("a benchmark case requires plain, plain-cached, signals, cached, or raw"),
        };
        let requested = args.get(index + 2).map(String::as_str).unwrap_or("all");
        let scenario = SCENARIOS
            .into_iter()
            .chain(["signals"])
            .find(|scenario| *scenario == requested)
            .expect("unknown benchmark scenario");
        let instances = args
            .get(index + 3)
            .map(|value| value.parse().unwrap())
            .unwrap_or(100);
        let signals = args
            .get(index + 4)
            .map(|value| value.parse().unwrap())
            .unwrap_or(1);
        assert!(instances > 0 && signals > 0);

        if args[index] == "--headless" {
            measure(variant, instances, signals, scenario, 31);
            return;
        }

        gpui_platform::application().run(move |cx| {
            cx.open_window(gpui::WindowOptions::default(), move |_, cx| {
                cx.new(|cx| {
                    let host = cx.new(|_| {
                        let mut host = Host::new(variant, signals);
                        for _ in 0..instances {
                            host.append();
                        }
                        host
                    });
                    Native {
                        host,
                        samples: Rc::default(),
                        scenario,
                    }
                })
            })
            .unwrap();
            cx.activate(true);
        });
        return;
    }

    let quick = args.iter().any(|arg| arg == "--quick");
    let sizes: &[usize] = if quick {
        &[1, 100]
    } else {
        &[1, 100, 1_000, 10_000]
    };
    let samples = if quick { 7 } else { 31 };

    for variant in [
        Variant::Plain,
        Variant::PlainCached,
        Variant::Signals,
        Variant::Cached,
        Variant::Raw,
    ] {
        for &count in sizes {
            for scenario in SCENARIOS {
                measure(variant, count, 1, scenario, samples);
            }
        }

        for signals in [1, 10, 50] {
            measure(variant, 100, signals, "signals", samples);
        }

        if !matches!(variant, Variant::Cached | Variant::PlainCached) {
            async_work::measure(variant, false, samples);
            async_work::measure(variant, true, samples);
        }
    }
    dependencies::run(samples);
    resources::run(samples);
}
