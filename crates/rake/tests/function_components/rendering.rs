use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rake::gpui::{self, Context, IntoElement, Render, Window, prelude::*};
use rake::reactive::{ReactiveApp, Setup};

use super::{Probe, change, draw};

#[test]
fn automatic_cache_preserves_positioned_roots_and_sibling_flow() {
    #[derive(Default, PartialEq)]
    struct BoundsProbe {
        positioned: Cell<gpui::Bounds<gpui::Pixels>>,
        sibling: Cell<gpui::Bounds<gpui::Pixels>>,
    }

    #[rake::component]
    fn Positioned(probe: Rc<BoundsProbe>, cx: &mut Setup) -> impl IntoElement {
        let _state = cx.signal(0);
        rake::view! {
            <div class="absolute left-4 top-4 w-40 h-10">
                {
                    gpui::canvas(
                        move |bounds, _, _| probe.positioned.set(bounds),
                        |_, _, _, _| {}
                    )
                    .size_full()
                }
            </div>
        }
    }

    struct Parent(Rc<BoundsProbe>);
    impl Render for Parent {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let probe = self.0.clone();
            gpui::div()
                .relative()
                .flex()
                .w(gpui::px(400.))
                .h(gpui::px(200.))
                .child(
                    Positioned::builder()
                        .id("positioned")
                        .probe(probe.clone())
                        .build(),
                )
                .child(
                    gpui::canvas(
                        move |bounds, _, _| probe.sibling.set(bounds),
                        |_, _, _, _| {},
                    )
                    .w(gpui::px(30.))
                    .h(gpui::px(40.)),
                )
        }
    }

    let mut cx = gpui::TestAppContext::single();
    let probe = Rc::new(BoundsProbe::default());
    let window = cx.add_window(|_, _| Parent(probe.clone()));
    for _ in 0..3 {
        change(&mut cx, window, |_| {});
        assert_eq!(
            probe.positioned.get().origin,
            gpui::point(gpui::px(16.), gpui::px(16.))
        );
        assert_eq!(
            probe.positioned.get().size,
            gpui::size(gpui::px(160.), gpui::px(40.))
        );
        assert_eq!(
            probe.sibling.get().origin,
            gpui::point(gpui::px(0.), gpui::px(0.))
        );
    }
}

#[test]
fn cached_siblings_skip_render_when_one_signal_changes() {
    #[rake::component(cached = "w-40 h-10")]
    fn Leaf(probe: Rc<Probe>, cx: &mut Setup) -> impl IntoElement {
        let value = cx.signal(0);
        probe.setter.set(Some(value));
        {
            probe.value.set(value.get(cx));
            probe.renders.set(probe.renders.get() + 1);
            gpui::div().w(gpui::px(160.)).h(gpui::px(40.)).child(value)
        }
    }
    struct Parent([Rc<Probe>; 2]);
    impl Render for Parent {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            gpui::div().children(
                self.0
                    .iter()
                    .enumerate()
                    .map(|(id, probe)| Leaf::builder().id(id).probe(probe.clone()).build()),
            )
        }
    }
    let mut cx = gpui::TestAppContext::single();
    let probes = [Rc::new(Probe::default()), Rc::new(Probe::default())];
    let window = cx.add_window(|_, _| Parent(probes.clone()));
    draw(&mut cx, window);
    let before = [probes[0].renders.get(), probes[1].renders.get()];
    for value in 1..5 {
        cx.update(|cx| probes[0].setter.get().unwrap().set(cx, value));
        draw(&mut cx, window);
        assert_eq!(probes[0].value.get(), value);
        assert_eq!(probes[0].renders.get(), before[0] + value as usize);
        assert_eq!(probes[1].renders.get(), before[1]);
    }
}

#[test]
fn state_changes_reflow_siblings_and_run_effects_after_paint() {
    #[derive(Default)]
    struct LayoutProbe {
        state: Probe,
        painted: Cell<bool>,
        events: RefCell<Vec<i32>>,
        cleanups: Cell<usize>,
        sibling_x: Cell<f32>,
    }

    #[rake::component]
    fn Resizable(probe: Rc<LayoutProbe>, cx: &mut Setup) -> impl IntoElement {
        let width = cx.signal(100);
        probe.state.setter.set(Some(width));
        let effect_probe = probe.clone();
        cx.effect(move |_, cx| {
            let width = width.get(cx);
            assert!(effect_probe.painted.get());
            effect_probe.events.borrow_mut().push(width);
            let cleanup = effect_probe.clone();
            cx.on_cleanup(move |_| cleanup.cleanups.set(cleanup.cleanups.get() + 1));
        });
        {
            let width = width.get(cx);
            probe.painted.set(false);
            gpui::canvas(|_, _, _| (), move |_, _, _, _| probe.painted.set(true))
                .w(gpui::px(width as f32))
                .h(gpui::px(40.))
                .flex_shrink_0()
        }
    }

    struct Parent(Rc<LayoutProbe>);
    impl Render for Parent {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let probe = self.0.clone();
            gpui::div()
                .flex()
                .child(Resizable {
                    id: "size".into(),
                    probe: probe.clone(),
                })
                .child(
                    gpui::canvas(
                        move |bounds, _, _| probe.sibling_x.set(bounds.origin.x.into()),
                        |_, _, _, _| {},
                    )
                    .w(gpui::px(30.))
                    .h(gpui::px(40.))
                    .flex_shrink_0(),
                )
        }
    }

    let mut cx = gpui::TestAppContext::single();
    let probe = Rc::new(LayoutProbe::default());
    let window = cx.add_window(|_, _| Parent(probe.clone()));
    draw(&mut cx, window);
    assert_eq!(probe.sibling_x.get(), 100.);
    cx.update(|cx| probe.state.setter.get().unwrap().set(cx, 200));
    draw(&mut cx, window);
    assert_eq!(probe.sibling_x.get(), 200.);
    assert_eq!(*probe.events.borrow(), [100, 200]);
    assert_eq!(probe.cleanups.get(), 1);
}

#[test]
fn virtualized_function_rows_release_state_when_scrolled_out() {
    #[rake::component]
    fn Row(probe: Rc<Probe>, cx: &mut Setup) -> impl IntoElement {
        let value = cx.signal(0);
        probe.setter.set(Some(value));
        {
            probe.value.set(value.get(cx));
            gpui::div().h(gpui::px(32.)).child(value)
        }
    }

    struct List {
        scroll: gpui::UniformListScrollHandle,
        probes: Rc<RefCell<std::collections::BTreeMap<usize, Rc<Probe>>>>,
    }
    impl Render for List {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            gpui::uniform_list(
                "rows",
                10_000,
                cx.processor(|this, range: std::ops::Range<usize>, _, _| {
                    range
                        .map(|id| Row {
                            id: id.into(),
                            probe: this.probes.borrow_mut().entry(id).or_default().clone(),
                        })
                        .collect()
                }),
            )
            .track_scroll(&self.scroll)
            .w(gpui::px(200.))
            .h(gpui::px(160.))
        }
    }

    let mut cx = gpui::TestAppContext::single();
    let probes = Rc::new(RefCell::new(std::collections::BTreeMap::new()));
    let window = cx.add_window(|_, _| List {
        scroll: gpui::UniformListScrollHandle::new(),
        probes: probes.clone(),
    });
    draw(&mut cx, window);
    let first = probes.borrow()[&1].clone();
    let stale = first.setter.get().unwrap();
    cx.update(|cx| stale.set(cx, 9));
    draw(&mut cx, window);
    assert_eq!(first.value.get(), 9);
    change(&mut cx, window, |list| {
        list.scroll.scroll_to_item(5000, gpui::ScrollStrategy::Top)
    });
    draw(&mut cx, window);
    assert!(
        cx.update(|cx| stale.update(cx, |value| *value = 1))
            .is_none()
    );
    assert!(probes.borrow().contains_key(&5000));
    assert!(probes.borrow().len() < 100);
    change(&mut cx, window, |list| {
        list.scroll.scroll_to_item(0, gpui::ScrollStrategy::Top)
    });
    assert_eq!(first.value.get(), 0);
}
