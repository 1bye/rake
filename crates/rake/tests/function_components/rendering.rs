use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rake::component::{ComponentError, Scope};
use rake::gpui::{self, Context, IntoElement, Render, Window, prelude::*};

use super::{Probe, change, draw};

#[test]
fn native_elements_isolate_leaf_updates() {
    #[rake::component]
    fn Leaf(cx: &mut Scope, probe: Rc<Probe>) -> impl IntoElement {
        let (value, setter) = cx.use_state(|| 0);
        probe.value.set(value);
        probe.setter.set(Some(setter));
        probe.renders.set(probe.renders.get() + 1);
        gpui::div()
            .w(gpui::px(100.))
            .h(gpui::px(40.))
            .child(value.to_string())
    }
    struct Parent(Rc<Cell<usize>>, [Rc<Probe>; 2]);
    impl Render for Parent {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.0.set(self.0.get() + 1);
            gpui::div().children(self.1.iter().enumerate().map(|(id, probe)| Leaf {
                id,
                probe: probe.clone(),
            }))
        }
    }
    let mut cx = gpui::TestAppContext::single();
    let renders = Rc::new(Cell::new(0));
    let probes = [Rc::new(Probe::default()), Rc::new(Probe::default())];
    let window = cx.add_window(|_, _| Parent(renders.clone(), probes.clone()));
    draw(&mut cx, window);
    let before = [
        renders.get(),
        probes[0].renders.get(),
        probes[1].renders.get(),
    ];
    cx.update(|cx| probes[0].setter.get().unwrap().set(cx, 1).unwrap());
    draw(&mut cx, window);
    assert_eq!(
        [
            renders.get(),
            probes[0].renders.get(),
            probes[1].renders.get()
        ],
        [before[0], before[1] + 1, before[2]]
    );
    for value in 2..5 {
        cx.update(|cx| probes[0].setter.get().unwrap().set(cx, value).unwrap());
        draw(&mut cx, window);
        assert_eq!(
            [
                renders.get(),
                probes[0].renders.get(),
                probes[1].renders.get()
            ],
            [before[0], before[1] + value as usize, before[2]]
        );
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
    fn Resizable(cx: &mut Scope, probe: Rc<LayoutProbe>) -> impl IntoElement {
        let (width, setter) = cx.use_state(|| 100);
        probe.state.setter.set(Some(setter));
        probe.painted.set(false);
        let effect_probe = probe.clone();
        cx.use_effect(width, move |_| {
            assert!(effect_probe.painted.get());
            effect_probe.events.borrow_mut().push(width);
            move || effect_probe.cleanups.set(effect_probe.cleanups.get() + 1)
        });
        gpui::canvas(|_, _, _| (), move |_, _, _, _| probe.painted.set(true))
            .w(gpui::px(width as f32))
            .h(gpui::px(40.))
            .flex_shrink_0()
    }

    struct Parent(Rc<LayoutProbe>);
    impl Render for Parent {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let probe = self.0.clone();
            gpui::div()
                .flex()
                .child(Resizable {
                    id: "size",
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
    cx.update(|cx| probe.state.setter.get().unwrap().set(cx, 200).unwrap());
    draw(&mut cx, window);
    assert_eq!(probe.sibling_x.get(), 200.);
    assert_eq!(*probe.events.borrow(), [100, 200]);
    assert_eq!(probe.cleanups.get(), 1);
}

#[test]
fn virtualized_function_rows_release_state_when_scrolled_out() {
    #[rake::component]
    fn Row(cx: &mut Scope, probe: Rc<Probe>) -> impl IntoElement {
        let (value, setter) = cx.use_state(|| 0);
        probe.value.set(value);
        probe.setter.set(Some(setter));
        gpui::div().h(gpui::px(32.)).child(value.to_string())
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
                            id,
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
    cx.update(|cx| stale.set(cx, 9).unwrap());
    draw(&mut cx, window);
    assert_eq!(first.value.get(), 9);
    change(&mut cx, window, |list| {
        list.scroll.scroll_to_item(5000, gpui::ScrollStrategy::Top)
    });
    draw(&mut cx, window);
    assert_eq!(
        cx.update(|cx| stale.set(cx, 1)),
        Err(ComponentError::Disposed)
    );
    assert!(probes.borrow().contains_key(&5000));
    assert!(probes.borrow().len() < 100);
    change(&mut cx, window, |list| {
        list.scroll.scroll_to_item(0, gpui::ScrollStrategy::Top)
    });
    assert_eq!(first.value.get(), 0);
}
